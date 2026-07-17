[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DataPath,

    [ValidateRange(1, 20)]
    [int]$Rounds = 3,

    [ValidateRange(1, 100000)]
    [int]$RequestCount = 300,

    [ValidateRange(1, 64)]
    [int]$Concurrency = 4,

    [ValidateRange(0, 10000)]
    [int]$WarmupRequestCount = 20,

    [ValidateRange(5, 3600)]
    [int]$MemoryDurationSeconds = 30,

    [ValidateRange(0, 3600)]
    [int]$MemoryWarmupSeconds = 5,

    [string]$ComposeFile = (Join-Path $PSScriptRoot "..\docker-compose.yml"),

    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$MEBIBYTE = 1024 * 1024
$LATENCY_DELTA_LIMIT_MILLISECONDS = 2.0
$LATENCY_DELTA_LIMIT_PERCENT = 15.0
$MEMORY_P95_LIMIT_MIB = 20.0
$MEMORY_PEAK_LIMIT_MIB = 24.0
$MEMORY_DELTA_LIMIT_MIB = 8.0
$EXPECTED_MEMORY_LIMIT_MIB = 160.0
$PROFILE_PATH = "/api/logging-profile-missing"
$REQUIRED_EVENT_FIELDS = @("timestamp", "level", "target", "event", "component")

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

function Get-Percentile {
    param(
        [Parameter(Mandatory = $true)]
        [double[]]$Values,

        [Parameter(Mandatory = $true)]
        [ValidateRange(0, 100)]
        [double]$Percentile
    )

    if ($Values.Count -eq 0) {
        throw "Cannot calculate a percentile from an empty sample"
    }
    $ordered = @($Values | Sort-Object)
    $rank = [Math]::Ceiling(($Percentile / 100.0) * $ordered.Count) - 1
    $index = [Math]::Max(0, [Math]::Min($ordered.Count - 1, $rank))
    [double]$ordered[$index]
}

function Wait-ContainerHealthy {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ContainerName,

        [ValidateRange(1, 300)]
        [int]$TimeoutSeconds = 90
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $state = Invoke-DockerCommand -Arguments @(
            "inspect",
            "--format",
            "{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}",
            $ContainerName
        ) -AllowFailure
        if ($state.ExitCode -eq 0 -and $state.Output -eq "healthy") {
            return
        }
        if ($state.ExitCode -eq 0 -and $state.Output -in @("exited", "dead")) {
            $logs = Invoke-DockerCommand -Arguments @("logs", $ContainerName) -AllowFailure
            throw "Container $ContainerName stopped before becoming healthy: $($logs.Output)"
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)

    $logs = Invoke-DockerCommand -Arguments @("logs", $ContainerName) -AllowFailure
    throw "Container $ContainerName did not become healthy: $($logs.Output)"
}

function Invoke-RequestBatch {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ContainerName,

        [Parameter(Mandatory = $true)]
        [int]$Count,

        [Parameter(Mandatory = $true)]
        [int]$WorkerCount
    )

    if ($Count -eq 0) {
        return [double[]]@()
    }
    $effectiveWorkerCount = [Math]::Min($Count, $WorkerCount)
    $script = @'
set -eu
request_count="$1"
worker_count="$2"
request_url="$3"
worker_index=0
while [ "$worker_index" -lt "$worker_count" ]
do
    (
        request_index="$worker_index"
        while [ "$request_index" -lt "$request_count" ]
        do
            curl --silent --show-error --output /dev/null --write-out '%{http_code} %{time_total}\n' "$request_url"
            request_index=$((request_index + worker_count))
        done
    ) &
    worker_index=$((worker_index + 1))
done
wait
'@
    $result = Invoke-DockerCommand -Arguments @(
        "exec",
        $ContainerName,
        "sh",
        "-c",
        $script,
        "profile-requests",
        $Count.ToString([Globalization.CultureInfo]::InvariantCulture),
        $effectiveWorkerCount.ToString([Globalization.CultureInfo]::InvariantCulture),
        "http://127.0.0.1:8000$PROFILE_PATH"
    )
    $latencies = [Collections.Generic.List[double]]::new()
    foreach ($line in ($result.Output -split "`r?`n")) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        if ($line -notmatch '^(\d{3})\s+([0-9.]+)$') {
            throw "Unexpected request sample: $line"
        }
        if ($Matches[1] -ne "404") {
            throw "Logging profile expected HTTP 404 but received $($Matches[1])"
        }
        $seconds = [double]::Parse(
            $Matches[2],
            [Globalization.CultureInfo]::InvariantCulture
        )
        $latencies.Add($seconds * 1000.0)
    }
    if ($latencies.Count -ne $Count) {
        throw "Expected $Count request samples but captured $($latencies.Count)"
    }
    [double[]]$latencies.ToArray()
}

function Convert-ApplicationLogs {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Text,

        [Parameter(Mandatory = $true)]
        [string]$Mode,

        [Parameter(Mandatory = $true)]
        [int]$ExpectedRequestEvents
    )

    $events = [Collections.Generic.List[object]]::new()
    $invalidLines = [Collections.Generic.List[string]]::new()
    $missingFieldLines = [Collections.Generic.List[string]]::new()
    foreach ($line in ($Text -split "`r?`n")) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }
        try {
            $event = $line | ConvertFrom-Json -ErrorAction Stop
        }
        catch {
            $invalidLines.Add($line)
            continue
        }
        $fieldNames = @($event.PSObject.Properties.Name)
        $eventNameProperty = $event.PSObject.Properties["event"]
        $eventName = if ($null -ne $eventNameProperty) {
            $eventNameProperty.Value
        }
        else {
            $null
        }
        $requiredFields = if ($eventName -eq "logging.events_dropped") {
            @("level", "target", "event", "component", "dropped_count")
        }
        else {
            $REQUIRED_EVENT_FIELDS
        }
        $missingFields = @($requiredFields | Where-Object { $_ -notin $fieldNames })
        if ($missingFields.Count -gt 0) {
            $missingFieldLines.Add(($missingFields -join ","))
        }
        $events.Add($event)
    }

    $requestEvents = @(
        $events |
            Where-Object {
                $null -ne $_.PSObject.Properties["event"] -and
                $_.PSObject.Properties["event"].Value -eq "http.request.completed"
            }
    )
    $dropEvents = @(
        $events |
            Where-Object {
                $null -ne $_.PSObject.Properties["event"] -and
                $_.PSObject.Properties["event"].Value -eq "logging.events_dropped"
            }
    )
    $droppedCount = [long](
        $dropEvents |
            ForEach-Object {
                $property = $_.PSObject.Properties["dropped_count"]
                if ($null -ne $property) {
                    [long]$property.Value
                }
            } |
            Measure-Object -Sum
    ).Sum
    [pscustomobject]@{
        LineCount = $events.Count + $invalidLines.Count
        RequestEventCount = $requestEvents.Count
        ExpectedRequestEventCount = if ($Mode -eq "default") {
            $ExpectedRequestEvents
        }
        else {
            0
        }
        DropEventCount = $dropEvents.Count
        DroppedLineCount = $droppedCount
        JsonLineCount = $events.Count
        InvalidLineCount = $invalidLines.Count
        MissingRequiredFieldCount = $missingFieldLines.Count
    }
}

function New-LoggingOverride {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$ResolvedDataPath,

        [Parameter(Mandatory = $true)]
        [ValidateSet("off", "default")]
        [string]$Mode
    )

    $yamlDataPath = $ResolvedDataPath.Replace("\", "/").Replace("'", "''")
    $override = @"
services:
  litradar:
    restart: "no"
    volumes:
      - '${yamlDataPath}:/app/data:rw'
    environment:
      LITRADAR_LOG_FORMAT: 'json'
"@
    if ($Mode -eq "off") {
        $override += "`n      LITRADAR_LOG_FILTER: 'off'`n"
    }
    else {
        $override += "`n"
    }
    [IO.File]::WriteAllText($Path, $override, [Text.UTF8Encoding]::new($false))
}

function Invoke-LatencyRun {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Mode,

        [Parameter(Mandatory = $true)]
        [int]$Round,

        [Parameter(Mandatory = $true)]
        [string]$ResolvedComposeFile,

        [Parameter(Mandatory = $true)]
        [string]$ResolvedDataPath,

        [Parameter(Mandatory = $true)]
        [string]$WorkingDirectory,

        [Parameter(Mandatory = $true)]
        [string]$Timestamp
    )

    $projectName = "litradar-logging-$Mode-$PID-$Round-$Timestamp".ToLowerInvariant()
    $containerName = "$projectName-container"
    $overridePath = Join-Path $WorkingDirectory "$projectName.compose.yaml"
    New-LoggingOverride -Path $overridePath -ResolvedDataPath $ResolvedDataPath -Mode $Mode
    $composePrefix = @(
        "compose",
        "--project-name", $projectName,
        "--file", $ResolvedComposeFile,
        "--file", $overridePath
    )
    $containerCreated = $false
    try {
        Invoke-DockerCommand -Arguments ($composePrefix + @(
            "run", "--no-deps", "--detach", "--name", $containerName, "litradar"
        )) | Out-Null
        $containerCreated = $true
        Wait-ContainerHealthy -ContainerName $containerName
        Invoke-RequestBatch `
            -ContainerName $containerName `
            -Count $WarmupRequestCount `
            -WorkerCount $Concurrency | Out-Null
        $latencies = Invoke-RequestBatch `
            -ContainerName $containerName `
            -Count $RequestCount `
            -WorkerCount $Concurrency
        Invoke-DockerCommand -Arguments @("stop", "--time", "15", $containerName) | Out-Null
        $logs = Invoke-DockerCommand -Arguments @("logs", $containerName)
        $logSummary = Convert-ApplicationLogs `
            -Text $logs.Output `
            -Mode $Mode `
            -ExpectedRequestEvents ($WarmupRequestCount + $RequestCount)
        [pscustomobject]@{
            Mode = $Mode
            Round = $Round
            RequestCount = $latencies.Count
            P50Milliseconds = [Math]::Round((Get-Percentile -Values $latencies -Percentile 50), 3)
            P95Milliseconds = [Math]::Round((Get-Percentile -Values $latencies -Percentile 95), 3)
            PeakMilliseconds = [Math]::Round(
                [double]($latencies | Measure-Object -Maximum).Maximum,
                3
            )
            LatencyMilliseconds = $latencies
            Logs = $logSummary
        }
    }
    finally {
        if ($containerCreated) {
            Invoke-DockerCommand -Arguments @("rm", "--force", $containerName) -AllowFailure |
                Out-Null
        }
        Invoke-DockerCommand -Arguments ($composePrefix + @("down", "--remove-orphans")) -AllowFailure |
            Out-Null
        if (Test-Path -LiteralPath $overridePath) {
            Remove-Item -LiteralPath $overridePath -Force
        }
    }
}

function Invoke-MemoryProfile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Mode,

        [Parameter(Mandatory = $true)]
        [string]$ResolvedComposeFile,

        [Parameter(Mandatory = $true)]
        [string]$ResolvedDataPath,

        [Parameter(Mandatory = $true)]
        [string]$WorkingDirectory,

        [Parameter(Mandatory = $true)]
        [string]$Timestamp
    )

    $overridePath = Join-Path $WorkingDirectory "$Timestamp-memory-$Mode.override.yaml"
    $renderedComposePath = Join-Path $WorkingDirectory "$Timestamp-memory-$Mode.compose.yaml"
    $memoryOutputPath = Join-Path $WorkingDirectory "$Timestamp-memory-$Mode.json"
    New-LoggingOverride -Path $overridePath -ResolvedDataPath $ResolvedDataPath -Mode $Mode
    try {
        $rendered = Invoke-DockerCommand -Arguments @(
            "compose",
            "--file", $ResolvedComposeFile,
            "--file", $overridePath,
            "config"
        )
        [IO.File]::WriteAllText(
            $renderedComposePath,
            $rendered.Output + [Environment]::NewLine,
            [Text.UTF8Encoding]::new($false)
        )
        $memoryProfiler = Join-Path $PSScriptRoot "profile_docker_memory.ps1"
        $profileRunner = @'
& {
    param(
        $ProfilerPath,
        $DataPath,
        $DurationSeconds,
        $WarmupSeconds,
        $SampleIntervalMilliseconds,
        $P95LimitMiB,
        $PeakLimitMiB,
        $ExpectedMemoryLimitMiB,
        $ComposeFile,
        $OutputPath
    )

    & $ProfilerPath `
        -Scenario "warm-idle" `
        -DataPath $DataPath `
        -DurationSeconds ([int]$DurationSeconds) `
        -WarmupSeconds ([int]$WarmupSeconds) `
        -SampleIntervalMilliseconds ([int]$SampleIntervalMilliseconds) `
        -P95LimitMiB ([double]$P95LimitMiB) `
        -PeakLimitMiB ([double]$PeakLimitMiB) `
        -ExpectedMemoryLimitMiB ([double]$ExpectedMemoryLimitMiB) `
        -TrafficPath @("/health/live", "/health/ready", "/") `
        -ComposeFile $ComposeFile `
        -OutputPath $OutputPath
}
'@
        $profileArguments = @(
            "-NoProfile",
            "-Command", $profileRunner,
            $memoryProfiler,
            $ResolvedDataPath,
            $MemoryDurationSeconds,
            $MemoryWarmupSeconds,
            "1000",
            $MEMORY_P95_LIMIT_MIB,
            $MEMORY_PEAK_LIMIT_MIB,
            $EXPECTED_MEMORY_LIMIT_MIB,
            $renderedComposePath,
            $memoryOutputPath
        )
        $profileOutput = & pwsh @profileArguments 2>&1
        $profileExitCode = $LASTEXITCODE
        if (-not (Test-Path -LiteralPath $memoryOutputPath)) {
            $profileText = ($profileOutput | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
            throw "Memory profile did not produce a report: $profileText"
        }
        $summary = Get-Content -LiteralPath $memoryOutputPath -Raw | ConvertFrom-Json
        [pscustomobject]@{
            Mode = $Mode
            ReportPath = $memoryOutputPath
            ExitCode = $profileExitCode
            WorkingSetP95Bytes = [long]$summary.Memory.WorkingSetP95Bytes
            WorkingSetPeakBytes = [long]$summary.Memory.WorkingSetPeakBytes
            GatePassed = [bool]$summary.Gate.Passed
            GateFailures = @($summary.Gate.Failures)
        }
    }
    finally {
        foreach ($path in @($overridePath, $renderedComposePath)) {
            if (Test-Path -LiteralPath $path) {
                Remove-Item -LiteralPath $path -Force
            }
        }
    }
}

if ($Concurrency -gt $RequestCount) {
    throw "Concurrency cannot exceed RequestCount"
}

$resolvedDataPath = (Resolve-Path -LiteralPath $DataPath).Path
$resolvedComposeFile = (Resolve-Path -LiteralPath $ComposeFile).Path
$outputDirectory = Join-Path (Split-Path $resolvedComposeFile -Parent) "output\logging"
[IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
$timestamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $outputDirectory "$timestamp-profile.json"
}
elseif (-not [IO.Path]::IsPathRooted($OutputPath)) {
    $OutputPath = Join-Path (Get-Location) $OutputPath
}
[IO.Directory]::CreateDirectory((Split-Path $OutputPath -Parent)) | Out-Null

$runs = [Collections.Generic.List[object]]::new()
for ($round = 1; $round -le $Rounds; $round++) {
    $modeOrder = if ($round % 2 -eq 1) {
        @("off", "default")
    }
    else {
        @("default", "off")
    }
    foreach ($mode in $modeOrder) {
        $runs.Add((Invoke-LatencyRun `
            -Mode $mode `
            -Round $round `
            -ResolvedComposeFile $resolvedComposeFile `
            -ResolvedDataPath $resolvedDataPath `
            -WorkingDirectory $outputDirectory `
            -Timestamp $timestamp))
    }
}

$offLatencies = [double[]]@(
    $runs |
        Where-Object { $_.Mode -eq "off" } |
        ForEach-Object { $_.LatencyMilliseconds }
)
$defaultLatencies = [double[]]@(
    $runs |
        Where-Object { $_.Mode -eq "default" } |
        ForEach-Object { $_.LatencyMilliseconds }
)
$offP95 = Get-Percentile -Values $offLatencies -Percentile 95
$defaultP95 = Get-Percentile -Values $defaultLatencies -Percentile 95
$latencyDelta = $defaultP95 - $offP95
$latencyDeltaPercent = if ($offP95 -eq 0.0) {
    0.0
}
else {
    ($latencyDelta / $offP95) * 100.0
}
$allowedLatencyDelta = [Math]::Max(
    $LATENCY_DELTA_LIMIT_MILLISECONDS,
    $offP95 * ($LATENCY_DELTA_LIMIT_PERCENT / 100.0)
)

$offMemory = Invoke-MemoryProfile `
    -Mode off `
    -ResolvedComposeFile $resolvedComposeFile `
    -ResolvedDataPath $resolvedDataPath `
    -WorkingDirectory $outputDirectory `
    -Timestamp $timestamp
$defaultMemory = Invoke-MemoryProfile `
    -Mode default `
    -ResolvedComposeFile $resolvedComposeFile `
    -ResolvedDataPath $resolvedDataPath `
    -WorkingDirectory $outputDirectory `
    -Timestamp $timestamp
$memoryDeltaBytes = $defaultMemory.WorkingSetP95Bytes - $offMemory.WorkingSetP95Bytes

$gateFailures = [Collections.Generic.List[string]]::new()
if ($latencyDelta -gt $allowedLatencyDelta) {
    $gateFailures.Add(
        "logging-on p95 latency delta exceeds max(2 ms, 15 percent)"
    )
}
$droppedLineCount = [long](
    $runs |
        ForEach-Object { $_.Logs.DroppedLineCount } |
        Measure-Object -Sum
).Sum
if ($droppedLineCount -ne 0) {
    $gateFailures.Add("expected-load dropped line count is $droppedLineCount")
}
$unexpectedOffLineCount = [long](
    $runs |
        Where-Object { $_.Mode -eq "off" } |
        ForEach-Object { $_.Logs.LineCount } |
        Measure-Object -Sum
).Sum
if ($unexpectedOffLineCount -ne 0) {
    $gateFailures.Add("logging-off emitted $unexpectedOffLineCount application lines")
}
$invalidLineCount = [long](
    $runs |
        ForEach-Object { $_.Logs.InvalidLineCount } |
        Measure-Object -Sum
).Sum
if ($invalidLineCount -ne 0) {
    $gateFailures.Add("captured $invalidLineCount non-JSON application lines")
}
$missingRequiredFieldCount = [long](
    $runs |
        ForEach-Object { $_.Logs.MissingRequiredFieldCount } |
        Measure-Object -Sum
).Sum
if ($missingRequiredFieldCount -ne 0) {
    $gateFailures.Add(
        "captured $missingRequiredFieldCount application events with missing required fields"
    )
}
$requestEventMismatchCount = @(
    $runs |
        Where-Object {
            $_.Mode -eq "default" -and
            $_.Logs.RequestEventCount -ne $_.Logs.ExpectedRequestEventCount
        }
).Count
if ($requestEventMismatchCount -ne 0) {
    $gateFailures.Add("$requestEventMismatchCount logging-on runs lost request events")
}
if ($offMemory.ExitCode -ne 0 -or -not $offMemory.GatePassed) {
    $gateFailures.Add("logging-off warm-idle memory profile failed")
}
if ($defaultMemory.ExitCode -ne 0 -or -not $defaultMemory.GatePassed) {
    $gateFailures.Add("logging-on warm-idle memory profile failed")
}
if ($memoryDeltaBytes -gt $MEMORY_DELTA_LIMIT_MIB * $MEBIBYTE) {
    $gateFailures.Add("logging-on p95 memory delta exceeds 8 MiB")
}

$finishedAt = [DateTime]::UtcNow
$summary = [ordered]@{
    SchemaVersion = 1
    StartedAtUtc = [DateTime]::ParseExact(
        $timestamp,
        "yyyyMMddTHHmmssfffZ",
        [Globalization.CultureInfo]::InvariantCulture,
        [Globalization.DateTimeStyles]::AssumeUniversal -bor
            [Globalization.DateTimeStyles]::AdjustToUniversal
    ).ToString("o")
    FinishedAtUtc = $finishedAt.ToString("o")
    Profile = [ordered]@{
        Rounds = $Rounds
        RequestCountPerRun = $RequestCount
        WarmupRequestCountPerRun = $WarmupRequestCount
        Concurrency = $Concurrency
        RequestPath = $PROFILE_PATH
        LoggingModes = @("off", "default-json")
    }
    Thresholds = [ordered]@{
        LatencyDeltaMilliseconds = $LATENCY_DELTA_LIMIT_MILLISECONDS
        LatencyDeltaPercent = $LATENCY_DELTA_LIMIT_PERCENT
        MemoryP95MiB = $MEMORY_P95_LIMIT_MIB
        MemoryPeakMiB = $MEMORY_PEAK_LIMIT_MIB
        MemoryDeltaMiB = $MEMORY_DELTA_LIMIT_MIB
        ContainerMemoryLimitMiB = $EXPECTED_MEMORY_LIMIT_MIB
    }
    Latency = [ordered]@{
        LoggingOffP95Milliseconds = [Math]::Round($offP95, 3)
        LoggingOnP95Milliseconds = [Math]::Round($defaultP95, 3)
        DeltaMilliseconds = [Math]::Round($latencyDelta, 3)
        DeltaPercent = [Math]::Round($latencyDeltaPercent, 3)
        AllowedDeltaMilliseconds = [Math]::Round($allowedLatencyDelta, 3)
    }
    Logging = [ordered]@{
        DroppedLineCount = $droppedLineCount
        RequiredEventFields = $REQUIRED_EVENT_FIELDS
        Runs = @($runs)
    }
    Memory = [ordered]@{
        LoggingOff = $offMemory
        LoggingOn = $defaultMemory
        P95DeltaBytes = $memoryDeltaBytes
        P95DeltaMiB = [Math]::Round($memoryDeltaBytes / $MEBIBYTE, 3)
    }
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
$summary | ConvertTo-Json -Depth 10 -Compress
if (-not $summary.Gate.Passed) {
    exit 1
}
