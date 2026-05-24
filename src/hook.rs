use std::io::Read;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use tracing::{debug, info};

use crate::error::AppError;

const MAX_SLEEP_SECS: u64 = 5 * 3600;

pub fn run_stop_failure(
    dry_run: bool,
    unknown_error_rewake_in_secs: Option<u64>,
) -> Result<ExitCode, AppError> {
    let mut stdin_str = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin_str);
    debug!("stop-failure stdin: {}", stdin_str.trim());

    match crate::statusline::lua_engine::run_quota_check()? {
        Some(reset_in) => {
            if dry_run {
                println!("quota exhausted, would sleep {reset_in}s then exit 2");
                return Ok(ExitCode::SUCCESS);
            }
            info!("quota exhausted, sleeping {reset_in}s until reset");
            eprintln!("quota exhausted, sleeping {reset_in}s until reset");
            thread::sleep(Duration::from_secs(reset_in.min(MAX_SLEEP_SECS)));
            Ok(ExitCode::from(2))
        }
        None => match unknown_error_rewake_in_secs {
            Some(secs) => {
                if dry_run {
                    println!("no exhausted quota, would sleep {secs}s then exit 2");
                    return Ok(ExitCode::SUCCESS);
                }
                info!("no exhausted quota, sleeping {secs}s (fallback rewake)");
                eprintln!("sleeping {secs}s before retry");
                thread::sleep(Duration::from_secs(secs.min(MAX_SLEEP_SECS)));
                Ok(ExitCode::from(2))
            }
            None => {
                debug!("no exhausted quota window and no fallback rewake configured");
                Ok(ExitCode::SUCCESS)
            }
        },
    }
}
