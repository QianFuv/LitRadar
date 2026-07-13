[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("warm-idle", "index", "update", "scheduled-child")]
    [string]$Scenario,

    [Parameter(Mandatory = $true)]
    [string]$DataPath,

    [string[]]$Command = @(),

    [ValidateRange(1, 86400)]
    [int]$DurationSeconds = 60,

    [ValidateRange(0, 3600)]
    [int]$WarmupSeconds = 0,

    [ValidateRange(100, 60000)]
    [int]$SampleIntervalMilliseconds = 1000,

    [Nullable[double]]$P95LimitMiB,

    [Nullable[double]]$PeakLimitMiB,

    [Nullable[double]]$ExpectedMemoryLimitMiB,

    [string[]]$TrafficPath = @(),

    [string]$ComposeFile = (Join-Path $PSScriptRoot "..\docker-compose.yml"),

    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$MEBIBYTE = 1024 * 1024
$DAILY_P95_LIMIT_MIB = 20.0
$DAILY_PEAK_LIMIT_MIB = 24.0
$JOB_P95_LIMIT_MIB = 100.0
$JOB_PEAK_LIMIT_MIB = 120.0

function Invoke-DockerCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,

        [switch]$AllowFailure
    )

    $output = & docker @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($output | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
    if ($exitCode -ne 0 -and -not $AllowFailure) {
        throw "Docker command failed with exit code ${exitCode}: $text"
    }
    [pscustomobject]@{
        ExitCode = $exitCode
        Output = $text.Trim()
    }
}

function Convert-KeyValueLines {
    param(
        [string[]]$Lines
    )

    $values = [ordered]@{}
    foreach ($line in $Lines) {
        if ($line -match '^([^\s]+)\s+(\d+)$') {
            $values[$Matches[1]] = [long]$Matches[2]
        }
    }
    $values
}

function Convert-PressureLines {
    param(
        [string[]]$Lines
    )

    $pressure = [ordered]@{}
    foreach ($line in $Lines) {
        if ($line -notmatch '^(some|full)\s+(.+)$') {
            continue
        }
        $kind = $Matches[1]
        $metrics = [ordered]@{}
        foreach ($field in $Matches[2] -split '\s+') {
            $parts = $field -split '=', 2
            if ($parts.Count -ne 2) {
                continue
            }
            if ($parts[0] -eq "total") {
                $metrics[$parts[0]] = [long]$parts[1]
            }
            else {
                $metrics[$parts[0]] = [double]::Parse(
                    $parts[1],
                    [Globalization.CultureInfo]::InvariantCulture
                )
            }
        }
        $pressure[$kind] = $metrics
    }
    $pressure
}

function Get-ContainerProcesses {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ContainerName
    )

    $result = Invoke-DockerCommand -Arguments @(
        "top", $ContainerName, "-eo", "pid,ppid,rss,nlwp,comm"
    ) -AllowFailure
    if ($result.ExitCode -ne 0) {
        return @()
    }
    $processes = @()
    foreach ($line in ($result.Output -split "`r?`n" | Select-Object -Skip 1)) {
        if ($line -notmatch '^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+(.+?)\s*$') {
            continue
        }
        $processes += [pscustomobject]@{
            Pid = [int]$Matches[1]
            ParentPid = [int]$Matches[2]
            RssBytes = [long]$Matches[3] * 1024
            ThreadCount = [int]$Matches[4]
            Command = $Matches[5]
        }
    }
    $processes
}

function Get-CgroupSnapshot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ContainerName
    )

    $probe = @'
set -eu
for entry in memory.current memory.peak memory.swap.current memory.max memory.events memory.pressure memory.stat
do
    printf '__%s__\n' "$entry"
    cat "/sys/fs/cgroup/$entry"
done
'@
    $result = Invoke-DockerCommand -Arguments @(
        "exec", $ContainerName, "sh", "-c", $probe
    ) -AllowFailure
    if ($result.ExitCode -ne 0) {
        return $null
    }

    $sections = @{}
    $sectionName = $null
    foreach ($line in ($result.Output -split "`r?`n")) {
        if ($line -match '^__(memory\.[a-z.]+)__$') {
            $sectionName = $Matches[1]
            $sections[$sectionName] = [Collections.Generic.List[string]]::new()
            continue
        }
        if ($null -ne $sectionName) {
            $sections[$sectionName].Add($line)
        }
    }
    foreach ($requiredSection in @(
        "memory.current",
        "memory.peak",
        "memory.swap.current",
        "memory.max",
        "memory.events",
        "memory.pressure",
        "memory.stat"
    )) {
        if (-not $sections.ContainsKey($requiredSection)) {
            throw "Missing cgroup section $requiredSection in profiler response"
        }
    }

    $memoryMaxText = $sections["memory.max"][0]
    $memoryMaxBytes = if ($memoryMaxText -eq "max") {
        $null
    }
    else {
        [long]$memoryMaxText
    }
    $stat = Convert-KeyValueLines -Lines $sections["memory.stat"]
    $selectedStat = [ordered]@{}
    foreach ($key in @(
        "anon",
        "file",
        "inactive_file",
        "kernel",
        "pagetables",
        "sock",
        "slab"
    )) {
        if ($stat.Contains($key)) {
            $selectedStat[$key] = $stat[$key]
        }
    }

    [pscustomobject]@{
        CurrentBytes = [long]$sections["memory.current"][0]
        CgroupPeakBytes = [long]$sections["memory.peak"][0]
        SwapBytes = [long]$sections["memory.swap.current"][0]
        MemoryMaxBytes = $memoryMaxBytes
        Events = Convert-KeyValueLines -Lines $sections["memory.events"]
        Pressure = Convert-PressureLines -Lines $sections["memory.pressure"]
        Stat = $selectedStat
        Processes = @(Get-ContainerProcesses -ContainerName $ContainerName)
    }
}

function Get-ContainerState {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ContainerName
    )

    $result = Invoke-DockerCommand -Arguments @(
        "inspect",
        "--format",
        "{{.State.Running}}|{{.State.ExitCode}}|{{.State.OOMKilled}}|{{if .State.Health}}{{.State.Health.Status}}{{end}}",
        $ContainerName
    ) -AllowFailure
    if ($result.ExitCode -ne 0) {
        return $null
    }
    $parts = $result.Output -split '\|', 4
    [pscustomobject]@{
        IsRunning = $parts[0] -eq "true"
        ExitCode = [int]$parts[1]
        OomKilled = $parts[2] -eq "true"
        Health = if ($parts.Count -eq 4) { $parts[3] } else { "" }
    }
}

function Wait-ContainerHealthy {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ContainerName,

        [int]$TimeoutSeconds = 90
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $state = Get-ContainerState -ContainerName $ContainerName
        if ($null -eq $state) {
            throw "Profiler container disappeared before readiness"
        }
        if (-not $state.IsRunning) {
            throw "Profiler container exited before readiness with code $($state.ExitCode)"
        }
        if ($state.Health -eq "healthy") {
            return
        }
        if ($state.Health -eq "unhealthy") {
            throw "Profiler container became unhealthy"
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Profiler container did not become healthy within $TimeoutSeconds seconds"
}

function Get-Percentile {
    param(
        [Parameter(Mandatory = $true)]
        [long[]]$Values,

        [Parameter(Mandatory = $true)]
        [ValidateRange(0.0, 100.0)]
        [double]$Percentile
    )

    if ($Values.Count -eq 0) {
        return 0L
    }
    $sorted = @($Values | Sort-Object)
    $index = [Math]::Max(
        0,
        [Math]::Ceiling(($Percentile / 100.0) * $sorted.Count) - 1
    )
    [long]$sorted[$index]
}

function Get-EventDelta {
    param(
        [Parameter(Mandatory = $true)]
        [Collections.IDictionary]$First,

        [Parameter(Mandatory = $true)]
        [Collections.IDictionary]$Last
    )

    $delta = [ordered]@{}
    foreach ($key in $Last.Keys) {
        $firstValue = if ($First.Contains($key)) { [long]$First[$key] } else { 0L }
        $delta[$key] = [long]$Last[$key] - $firstValue
    }
    $delta
}

function Start-BackgroundDockerExec {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ContainerName,

        [Parameter(Mandatory = $true)]
        [string[]]$ChildCommand
    )

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = "docker"
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in @("exec", $ContainerName, "/usr/local/bin/litradar") + $ChildCommand) {
        $startInfo.ArgumentList.Add($argument)
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw "Could not start scheduled-child docker exec"
    }
    $process.BeginOutputReadLine()
    $process.BeginErrorReadLine()
    $process
}

$isDailyScenario = $Scenario -eq "warm-idle"
$effectiveP95LimitMiB = if ($null -ne $P95LimitMiB) {
    [double]$P95LimitMiB
}
elseif ($isDailyScenario) {
    $DAILY_P95_LIMIT_MIB
}
else {
    $JOB_P95_LIMIT_MIB
}
$effectivePeakLimitMiB = if ($null -ne $PeakLimitMiB) {
    [double]$PeakLimitMiB
}
elseif ($isDailyScenario) {
    $DAILY_PEAK_LIMIT_MIB
}
else {
    $JOB_PEAK_LIMIT_MIB
}

if ($Scenario -in @("index", "update", "scheduled-child") -and $Command.Count -eq 0) {
    throw "Scenario '$Scenario' requires an explicit -Command argument array"
}
if ($Scenario -eq "index" -and $Command[0] -ne "index") {
    throw "The index scenario command must start with 'index'"
}
if ($Scenario -eq "update" -and ($Command[0] -ne "index" -or "--update" -notin $Command)) {
    throw "The update scenario requires an index command containing '--update'"
}

$resolvedDataPath = (Resolve-Path -LiteralPath $DataPath).Path
$resolvedComposeFile = (Resolve-Path -LiteralPath $ComposeFile).Path
$outputDirectory = Join-Path (Split-Path $resolvedComposeFile -Parent) "output\memory"
[IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
$timestamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $outputDirectory "$timestamp-$Scenario.json"
}
elseif (-not [IO.Path]::IsPathRooted($OutputPath)) {
    $OutputPath = Join-Path (Get-Location) $OutputPath
}
[IO.Directory]::CreateDirectory((Split-Path $OutputPath -Parent)) | Out-Null

$safeScenario = $Scenario.Replace("-", "")
$projectName = "litradar-memory-$safeScenario-$PID-$timestamp".ToLowerInvariant()
$containerName = "$projectName-container"
$networkName = "${projectName}_default"
$overridePath = Join-Path $outputDirectory "$projectName.compose.yaml"
$yamlDataPath = $resolvedDataPath.Replace("\", "/").Replace("'", "''")
$override = @"
services:
  litradar:
    restart: "no"
    volumes:
      - '${yamlDataPath}:/app/data:rw'
"@
[IO.File]::WriteAllText($overridePath, $override, [Text.UTF8Encoding]::new($false))

$composePrefix = @(
    "compose",
    "--project-name", $projectName,
    "--file", $resolvedComposeFile,
    "--file", $overridePath
)
$samples = [Collections.Generic.List[object]]::new()
$processPeaks = @{}
$childProcess = $null
$commandExitCode = $null
$trafficFailureCount = 0
$containerOomKilled = $false
$summary = $null
$containerCreated = $false
$measurementStartedAt = $null

try {
    $runArguments = $composePrefix + @(
        "run", "--no-deps", "--detach", "--name", $containerName, "litradar"
    )
    if ($Scenario -in @("index", "update")) {
        $runArguments += $Command
    }
    Invoke-DockerCommand -Arguments $runArguments | Out-Null
    $containerCreated = $true

    $inspectLimit = Invoke-DockerCommand -Arguments @(
        "inspect", "--format", "{{.HostConfig.Memory}}", $containerName
    )
    $containerMemoryLimitBytes = [long]$inspectLimit.Output

    if ($Scenario -in @("warm-idle", "scheduled-child")) {
        Wait-ContainerHealthy -ContainerName $containerName
    }
    if ($WarmupSeconds -gt 0) {
        Start-Sleep -Seconds $WarmupSeconds
    }
    if ($Scenario -eq "scheduled-child") {
        $childProcess = Start-BackgroundDockerExec `
            -ContainerName $containerName `
            -ChildCommand $Command
    }

    $measurementStartedAt = [DateTime]::UtcNow
    $deadline = $measurementStartedAt.AddSeconds($DurationSeconds)
    do {
        $state = Get-ContainerState -ContainerName $containerName
        if ($null -eq $state) {
            break
        }
        $containerOomKilled = $containerOomKilled -or $state.OomKilled
        if (-not $state.IsRunning) {
            $commandExitCode = $state.ExitCode
            break
        }
        if ($Scenario -eq "scheduled-child" -and $childProcess.HasExited) {
            $commandExitCode = $childProcess.ExitCode
        }

        $snapshot = Get-CgroupSnapshot -ContainerName $containerName
        if ($null -eq $snapshot) {
            break
        }
        $elapsedMilliseconds = [long]([DateTime]::UtcNow - $measurementStartedAt).TotalMilliseconds
        $inactiveFileBytes = if ($snapshot.Stat.Contains("inactive_file")) {
            [long]$snapshot.Stat["inactive_file"]
        }
        else {
            0L
        }
        $workingSetBytes = [Math]::Max(0L, $snapshot.CurrentBytes - $inactiveFileBytes)
        $processRssBytes = [long](
            $snapshot.Processes | Measure-Object -Property RssBytes -Sum
        ).Sum
        $threadCount = [int](
            $snapshot.Processes | Measure-Object -Property ThreadCount -Sum
        ).Sum
        foreach ($processEntry in $snapshot.Processes) {
            $key = "$($processEntry.Pid):$($processEntry.Command)"
            if (-not $processPeaks.ContainsKey($key) -or
                $processEntry.RssBytes -gt $processPeaks[$key].RssBytes) {
                $processPeaks[$key] = $processEntry
            }
        }
        $samples.Add([pscustomobject]@{
            ElapsedMilliseconds = $elapsedMilliseconds
            CurrentBytes = $snapshot.CurrentBytes
            WorkingSetBytes = $workingSetBytes
            CgroupPeakBytes = $snapshot.CgroupPeakBytes
            SwapBytes = $snapshot.SwapBytes
            ProcessRssBytes = $processRssBytes
            ProcessCount = $snapshot.Processes.Count
            ThreadCount = $threadCount
            Events = $snapshot.Events
            Pressure = $snapshot.Pressure
            Stat = $snapshot.Stat
        })

        foreach ($path in $TrafficPath) {
            $trafficResult = Invoke-DockerCommand -Arguments @(
                "exec",
                $containerName,
                "curl",
                "--fail",
                "--silent",
                "--output", "/dev/null",
                "http://127.0.0.1:8000$path"
            ) -AllowFailure
            if ($trafficResult.ExitCode -ne 0) {
                $trafficFailureCount++
            }
        }

        if ($Scenario -eq "scheduled-child" -and $null -ne $commandExitCode) {
            break
        }
        if ($Scenario -in @("index", "update")) {
            $stateAfterSample = Get-ContainerState -ContainerName $containerName
            if ($null -eq $stateAfterSample -or -not $stateAfterSample.IsRunning) {
                if ($null -ne $stateAfterSample) {
                    $commandExitCode = $stateAfterSample.ExitCode
                }
                break
            }
        }
        Start-Sleep -Milliseconds $SampleIntervalMilliseconds
    } while ([DateTime]::UtcNow -lt $deadline)

    if ($samples.Count -eq 0) {
        throw "Profiler captured no cgroup samples"
    }
    if ($null -eq $commandExitCode -and $Scenario -in @("index", "update")) {
        $state = Get-ContainerState -ContainerName $containerName
        if ($null -ne $state -and -not $state.IsRunning) {
            $containerOomKilled = $containerOomKilled -or $state.OomKilled
            $commandExitCode = $state.ExitCode
        }
        else {
            $commandExitCode = 124
        }
    }
    if ($null -eq $commandExitCode -and $Scenario -eq "scheduled-child") {
        if ($childProcess.HasExited) {
            $commandExitCode = $childProcess.ExitCode
        }
        else {
            $commandExitCode = 124
        }
    }
    if ($null -eq $commandExitCode) {
        $commandExitCode = 0
    }

    $currentValues = [long[]]@($samples | ForEach-Object { $_.CurrentBytes })
    $workingSetValues = [long[]]@($samples | ForEach-Object { $_.WorkingSetBytes })
    $workingSetP50Bytes = Get-Percentile -Values $workingSetValues -Percentile 50
    $workingSetP95Bytes = Get-Percentile -Values $workingSetValues -Percentile 95
    $workingSetPeakBytes = [long]($workingSetValues | Measure-Object -Maximum).Maximum
    $currentP50Bytes = Get-Percentile -Values $currentValues -Percentile 50
    $currentP95Bytes = Get-Percentile -Values $currentValues -Percentile 95
    $currentPeakBytes = [long]($currentValues | Measure-Object -Maximum).Maximum
    $cgroupPeakBytes = [long](
        $samples | Measure-Object -Property CgroupPeakBytes -Maximum
    ).Maximum
    $swapPeakBytes = [long](
        $samples | Measure-Object -Property SwapBytes -Maximum
    ).Maximum
    $lastEvents = $samples[$samples.Count - 1].Events
    $eventDelta = Get-EventDelta -First ([ordered]@{}) -Last $lastEvents
    $oomEventCount = 0L
    foreach ($eventName in @("oom", "oom_kill", "oom_group_kill")) {
        if ($eventDelta.Contains($eventName)) {
            $oomEventCount += [long]$eventDelta[$eventName]
        }
    }
    $fullPressureAvg10Max = [double](
        $samples |
            ForEach-Object {
                if ($_.Pressure.Contains("full") -and $_.Pressure["full"].Contains("avg10")) {
                    $_.Pressure["full"]["avg10"]
                }
                else {
                    0.0
                }
            } |
            Measure-Object -Maximum
    ).Maximum
    $peakProcesses = @(
        $processPeaks.Values |
            Sort-Object -Property RssBytes -Descending |
            Select-Object -First 10
    )

    $gateFailures = [Collections.Generic.List[string]]::new()
    if ($workingSetP95Bytes -gt $effectiveP95LimitMiB * $MEBIBYTE) {
        $gateFailures.Add("p95 memory exceeds $effectiveP95LimitMiB MiB")
    }
    if ($workingSetPeakBytes -gt $effectivePeakLimitMiB * $MEBIBYTE) {
        $gateFailures.Add("sample peak memory exceeds $effectivePeakLimitMiB MiB")
    }
    if ($swapPeakBytes -ne 0) {
        $gateFailures.Add("swap usage is nonzero")
    }
    if ($oomEventCount -ne 0) {
        $gateFailures.Add("OOM event delta is nonzero")
    }
    if ($containerOomKilled) {
        $gateFailures.Add("container OOM-killed state is true")
    }
    if ($eventDelta.Contains("max") -and [long]$eventDelta["max"] -ne 0) {
        $gateFailures.Add("memory.max event delta is nonzero")
    }
    if ($commandExitCode -ne 0) {
        $gateFailures.Add("scenario command exit code is $commandExitCode")
    }
    if ($trafficFailureCount -ne 0) {
        $gateFailures.Add("light-traffic request failures total $trafficFailureCount")
    }
    if ($fullPressureAvg10Max -ne 0.0) {
        $gateFailures.Add("memory full-pressure avg10 is nonzero")
    }
    if ($null -ne $ExpectedMemoryLimitMiB) {
        $expectedLimitBytes = [long]([double]$ExpectedMemoryLimitMiB * $MEBIBYTE)
        if ($containerMemoryLimitBytes -ne $expectedLimitBytes) {
            $gateFailures.Add(
                "container memory limit is $containerMemoryLimitBytes bytes, expected $expectedLimitBytes"
            )
        }
    }

    $finishedAt = [DateTime]::UtcNow
    $summary = [ordered]@{
        SchemaVersion = 1
        Scenario = $Scenario
        StartedAtUtc = $measurementStartedAt.ToString("o")
        FinishedAtUtc = $finishedAt.ToString("o")
        DurationSeconds = [Math]::Round(($finishedAt - $measurementStartedAt).TotalSeconds, 3)
        SampleIntervalMilliseconds = $SampleIntervalMilliseconds
        SampleCount = $samples.Count
        CommandProvided = $Command.Count -gt 0
        ExitCode = $commandExitCode
        ContainerMemoryLimitBytes = $containerMemoryLimitBytes
        Thresholds = [ordered]@{
            Metric = "cgroup current minus inactive_file"
            P95MiB = $effectiveP95LimitMiB
            PeakMiB = $effectivePeakLimitMiB
        }
        Memory = [ordered]@{
            WorkingSetP50Bytes = $workingSetP50Bytes
            WorkingSetP95Bytes = $workingSetP95Bytes
            WorkingSetPeakBytes = $workingSetPeakBytes
            CgroupCurrentP50Bytes = $currentP50Bytes
            CgroupCurrentP95Bytes = $currentP95Bytes
            CgroupCurrentPeakBytes = $currentPeakBytes
            CgroupLifetimePeakBytes = $cgroupPeakBytes
            SwapPeakBytes = $swapPeakBytes
        }
        EventDelta = $eventDelta
        OomDetected = $oomEventCount -ne 0 -or $containerOomKilled
        ContainerOomKilled = $containerOomKilled
        FullPressureAvg10Max = $fullPressureAvg10Max
        TrafficFailureCount = $trafficFailureCount
        PeakProcesses = $peakProcesses
        Samples = $samples
        Gate = [ordered]@{
            Passed = $gateFailures.Count -eq 0
            Failures = @($gateFailures)
        }
    }
    [IO.File]::WriteAllText(
        $OutputPath,
        ($summary | ConvertTo-Json -Depth 10 -Compress),
        [Text.UTF8Encoding]::new($false)
    )
}
finally {
    if ($null -ne $childProcess) {
        if (-not $childProcess.HasExited) {
            $childProcess.Kill($true)
            $childProcess.WaitForExit()
        }
        $childProcess.Dispose()
    }
    if ($containerCreated) {
        Invoke-DockerCommand -Arguments @("rm", "--force", $containerName) -AllowFailure |
            Out-Null
    }
    Invoke-DockerCommand -Arguments @("network", "rm", $networkName) -AllowFailure |
        Out-Null
    if (Test-Path -LiteralPath $overridePath) {
        Remove-Item -LiteralPath $overridePath -Force
    }
}

$summary | ConvertTo-Json -Depth 10 -Compress
if (-not $summary.Gate.Passed) {
    exit 1
}
