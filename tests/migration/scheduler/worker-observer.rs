/// Observe private pure scheduler behavior without modifying the original implementation.
pub fn observe(input: serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    match input["op"].as_str().unwrap() {
        "state" => {
            match serde_json::from_str::<SchedulerRunState>(input["raw"].as_str().unwrap()) {
                Ok(state) => {
                    json!({"encoded":serde_json::to_string(&state).unwrap(),"terminal":state.is_terminal()})
                }
                Err(_) => json!({"error":"json"}),
            }
        }
        "zones" => json!(
            chrono_tz::TZ_VARIANTS
                .iter()
                .map(|zone| zone.name())
                .collect::<Vec<_>>()
        ),
        "job" => {
            let parsed = serde_json::from_str::<ScheduledJobSpec>(input["raw"].as_str().unwrap());
            match parsed {
                Err(_) => json!({"error":"json"}),
                Ok(job) => {
                    json!({"encoded":serde_json::to_string(&job).unwrap(),"validation":job.validate().err().map(|error|error.to_string())})
                }
            }
        }
        "timing" => json!(
            validate_scheduled_task_timing(
                input["timezone"].as_str().unwrap(),
                input["timeout"].as_u64().unwrap()
            )
            .err()
            .map(|error| error.to_string())
        ),
        "cron" => {
            let expression = input["expression"].as_str().unwrap();
            if let Err(error) = validate_cron_expression(expression) {
                return json!({"error":error.to_string()});
            }
            let task:ScheduledTaskInfo=serde_json::from_value(json!({"id":1,"name":"fixture","job":{"kind":"index"},"legacy_command":null,"cron":expression,"timezone":input["timezone"],"timeout_seconds":60,"coalesce":true,"enabled":true,"last_run_at":null,"last_status":"idle","created_at":0.0,"updated_at":0.0})).unwrap();
            match scheduled_slots(
                &task,
                input["from"].as_f64().unwrap(),
                input["to"].as_f64().unwrap(),
            ) {
                Ok(slots) => json!({"slots":slots}),
                Err(error) => json!({"error":error.to_string()}),
            }
        }
        "commands" => {
            let job: ScheduledJobSpec = serde_json::from_value(input["job"].clone()).unwrap();
            match scheduled_processes(Path::new(input["root"].as_str().unwrap()),Path::new(input["auth"].as_str().unwrap()),Path::new(input["executable"].as_str().unwrap()),Path::new(input["key"].as_str().unwrap()),&job){Ok(commands)=>json!(commands.iter().map(|command|json!({"command":command.command,"path":command.executable.to_str().unwrap(),"arguments":command.arguments.iter().map(|value|value.to_str().unwrap()).collect::<Vec<_>>()})).collect::<Vec<_>>()),Err(error)=>json!({"error":error.to_string()})}
        }
        "summary" => {
            let terminal = match input["terminal"].as_str().unwrap() {
                "success" => ProcessTerminal::Success,
                "cancelled" => ProcessTerminal::Cancelled,
                "timeout" => ProcessTerminal::TimedOut,
                "heartbeat" => ProcessTerminal::HeartbeatLost,
                "exit" => {
                    ProcessTerminal::ExitFailed(input["code"].as_i64().map(|code| code as i32))
                }
                _ => ProcessTerminal::SupervisorFailed(ProcessSupervisorErrorKind::SpawnOrAssign),
            };
            let bytes = input["bytes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|byte| byte.as_u64().unwrap() as u8)
                .collect();
            let result = process_execution(terminal, "index", bytes);
            json!({"status":result.status,"summary":result.output_summary})
        }
        _ => panic!("unknown worker observer operation"),
    }
}
