use std::io::Read;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, SystemTime};

use tracing::{debug, info, warn};

use crate::error::AppError;

const MAX_SLEEP_SECS: u64 = 5 * 3600;
const SUSPEND_DRIFT_THRESHOLD_SECS: u64 = 60;

pub fn run_stop_failure(
    dry_run: bool,
    unknown_error_rewake_in_secs: Option<u64>,
    recheck_interval_seconds: u64,
) -> Result<ExitCode, AppError> {
    let mut stdin_str = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin_str);
    debug!("stop-failure stdin: {}", stdin_str.trim());

    let total_secs = match crate::statusline::lua_engine::run_quota_check()? {
        Some(reset_in) => {
            let secs = reset_in.min(MAX_SLEEP_SECS);
            if dry_run {
                println!("quota exhausted, would sleep {secs}s then exit 2");
                return Ok(ExitCode::SUCCESS);
            }
            info!("quota exhausted, sleeping {secs}s until reset");
            eprintln!("quota exhausted, sleeping {secs}s until reset");
            Some(secs)
        }
        None => match unknown_error_rewake_in_secs {
            Some(secs) => {
                let secs = secs.min(MAX_SLEEP_SECS);
                if dry_run {
                    println!("no exhausted quota, would sleep {secs}s then exit 2");
                    return Ok(ExitCode::SUCCESS);
                }
                info!("no exhausted quota, sleeping {secs}s (fallback rewake)");
                eprintln!("sleeping {secs}s before retry");
                Some(secs)
            }
            None => {
                debug!("no exhausted quota window and no fallback rewake configured");
                None
            }
        },
    };

    match total_secs {
        Some(secs) => {
            recheck_sleep(secs, recheck_interval_seconds);
            Ok(ExitCode::from(2))
        }
        None => Ok(ExitCode::SUCCESS),
    }
}

fn recheck_sleep(total_secs: u64, recheck_interval_seconds: u64) {
    let deadline = SystemTime::now() + Duration::from_secs(total_secs);
    let mut last_check = SystemTime::now();

    loop {
        let now = SystemTime::now();
        let remaining = deadline.duration_since(now).unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            return;
        }

        let requested_sleep = Duration::from_secs(recheck_interval_seconds).min(remaining);

        thread::sleep(requested_sleep);

        let woke_at = SystemTime::now();
        let actual_elapsed = woke_at.duration_since(last_check).unwrap_or(Duration::ZERO);
        let expected_elapsed = requested_sleep + Duration::from_secs(1); // allow 1s tolerance

        if actual_elapsed > expected_elapsed + Duration::from_secs(SUSPEND_DRIFT_THRESHOLD_SECS) {
            let drift = actual_elapsed.saturating_sub(expected_elapsed);
            warn!(
                actual_elapsed_secs = actual_elapsed.as_secs(),
                expected_elapsed_secs = expected_elapsed.as_secs(),
                drift_secs = drift.as_secs(),
                "possible suspend/resume detected, wall-clock drifted significantly"
            );
        }

        last_check = woke_at;
    }
}
