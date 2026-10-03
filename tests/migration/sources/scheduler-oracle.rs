//! Observe unchanged source scheduler state without waiting on a wall clock.
#![allow(dead_code)]
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::time::Duration;
include!("../../../output/migration/execution/sources-oracle/schedulers.rs");

fn duration(value: &Value) -> Duration {
    Duration::new(
        value["seconds"].as_str().unwrap_or("0").parse().unwrap(),
        value["nanoseconds"].as_u64().unwrap_or(0) as u32,
    )
}
fn encoded(value: Duration) -> Value {
    json!({"seconds":value.as_secs().to_string(),"nanoseconds":value.subsec_nanos()})
}
fn optional_duration(value: &Value) -> Option<Duration> {
    (!value.is_null()).then(|| duration(value))
}
fn unsigned(value: &Value) -> Option<u64> {
    value.as_str().map(|value| value.parse().unwrap())
}
fn slot(value: &Value) -> usize {
    value["slot"].as_u64().unwrap_or(0) as usize
}
fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        let key_count = request["keys"].as_u64().unwrap() as usize;
        let process_id = unsigned(&request["process_id"]).unwrap_or(0) as usize;
        let process_count = unsigned(&request["process_count"]).unwrap_or(1) as usize;
        let epoch = duration(&request["epoch"]);
        let mut oa = OpenAlexSchedulerState::with_context(
            key_count,
            process_id,
            process_count,
            epoch,
            unsigned(&request["capacity"]).unwrap_or(1) as usize,
        );
        let mut s2 = SemanticScholarSchedulerState::with_context(
            key_count,
            process_id,
            process_count,
            epoch,
            duration(&request["interval"]),
        );
        let is_oa = request["service"] == "openalex";
        let mut observations = Vec::new();
        for operation in request["operations"].as_array().unwrap() {
            let now = duration(&operation["now"]);
            let excluded: Vec<usize> =
                serde_json::from_value(operation["excluded"].clone()).unwrap_or_default();
            let outcome = operation["health"].as_str().unwrap_or("Success");
            let oa_reservation = OpenAlexSlotReservation {
                slot_index: slot(operation),
                start_at: duration(&operation["start"]),
            };
            let s2_reservation = SemanticScholarSlotReservation {
                slot_index: slot(operation),
                start_at: duration(&operation["start"]),
            };
            let result = match operation["op"].as_str().unwrap() {
                "reserve" if is_oa => match oa.reserve_slot(now, &excluded) {
                    OpenAlexScheduleDecision::Reserved(value) => {
                        json!({"kind":"Reserved","slot":value.slot_index,"start":encoded(value.start_at)})
                    }
                    OpenAlexScheduleDecision::WaitUntil(value) => {
                        json!({"kind":"WaitUntil","until":encoded(value)})
                    }
                    OpenAlexScheduleDecision::WaitForChange => json!({"kind":"WaitForChange"}),
                    OpenAlexScheduleDecision::Unavailable => json!({"kind":"Unavailable"}),
                },
                "reserve" => match s2.reserve_slot(now, &excluded) {
                    SemanticScholarScheduleDecision::Reserved(value) => {
                        json!({"kind":"Reserved","slot":value.slot_index,"start":encoded(value.start_at)})
                    }
                    SemanticScholarScheduleDecision::WaitUntil(value) => {
                        json!({"kind":"WaitUntil","until":encoded(value)})
                    }
                    SemanticScholarScheduleDecision::Unavailable => json!({"kind":"Unavailable"}),
                },
                "finish" if is_oa => {
                    let health = match outcome {
                        "Success" => OpenAlexHealthOutcome::Success,
                        "AuthenticationFailure" => OpenAlexHealthOutcome::AuthenticationFailure,
                        "RateLimited" => OpenAlexHealthOutcome::RateLimited,
                        "DailyQuotaLimited" => OpenAlexHealthOutcome::DailyQuotaLimited,
                        "TransientFailure" => OpenAlexHealthOutcome::TransientFailure,
                        _ => OpenAlexHealthOutcome::TerminalFailure,
                    };
                    oa.finish_slot(
                        &oa_reservation,
                        now,
                        OpenAlexRateHeaders {
                            remaining: unsigned(&operation["remaining"]),
                            credits_used: unsigned(&operation["credits"]),
                            reset_after: optional_duration(&operation["reset"]),
                            retry_after: optional_duration(&operation["retry"]),
                        },
                        health,
                        duration(&operation["delay"]),
                        if operation["search"] == true {
                            OpenAlexRequestClass::Search
                        } else {
                            OpenAlexRequestClass::List
                        },
                    );
                    Value::Null
                }
                "finish" => {
                    let health = match outcome {
                        "Success" => SemanticScholarHealthOutcome::Success,
                        "AuthenticationFailure" => {
                            SemanticScholarHealthOutcome::AuthenticationFailure
                        }
                        "RateLimited" => SemanticScholarHealthOutcome::RateLimited,
                        "TransientFailure" => SemanticScholarHealthOutcome::TransientFailure,
                        _ => SemanticScholarHealthOutcome::TerminalFailure,
                    };
                    s2.finish_slot(&s2_reservation, now, health, duration(&operation["delay"]));
                    Value::Null
                }
                "eligible" if is_oa => json!(oa.reservation_is_eligible(&oa_reservation, now)),
                "eligible" => json!(!s2.reservation_is_obsolete(&s2_reservation, now)),
                "cancel" => {
                    oa.cancel_slot(&oa_reservation);
                    Value::Null
                }
                _ => panic!("unknown operation"),
            };
            let state = if is_oa {
                json!({"next_tie":oa.next_tie_slot,"period":encoded(oa.period),"reserve":oa.daily_reserve_credits().to_string(),"slots":oa.slots.iter().map(|slot|json!({"next":encoded(slot.next_start_at),"cooldown":slot.cooldown_until.map(encoded),"disabled":slot.is_disabled,"remaining":slot.remaining.map(|value|value.to_string()),"reset":slot.reset_at.map(encoded),"in_flight":slot.in_flight.to_string(),"credit":slot.selection_credit.to_string()})).collect::<Vec<_>>()})
            } else {
                json!({"next_tie":s2.next_tie_slot,"period":encoded(s2.period),"slots":s2.slots.iter().map(|slot|json!({"next":encoded(slot.next_start_at),"cooldown":slot.cooldown_until.map(encoded),"disabled":slot.is_disabled})).collect::<Vec<_>>()})
            };
            observations.push(json!({"result":result,"state":state}));
        }
        request["output"] = json!(observations);
        writeln!(output, "{request}")?;
    }
    output.flush()
}
