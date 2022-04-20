use crate::constants;
use crate::config;

use lazy_static::lazy_static;
use log::{debug, error};
use regex::Regex;
use simple_error::bail;
use std::error::Error;
use std::process::Command;

pub fn fetch(lic: &config::FlexLM, lmstat: &str) -> Result<(), Box<dyn Error>> {
    lazy_static!{
        static ref RE_LMSTAT_USAGE: Regex = Regex::new(r"^Users of ([a-zA-Z0-9_\-+]+):\s+\(Total of (\d+) license[s]? issued;\s+Total of (\d+) license[s]? in use\)$").unwrap();
    }

    debug!("flexlm.rs:fetch: Running {} -c {} -a", lmstat, &lic.license);
    let cmd = Command::new(lmstat)
    .arg("-c")
    .arg(&lic.license)
    .arg("-a")
    .output()?;

    let rc = match cmd.status.code() {
        Some(v) => v,
        None => {
            bail!("Can't get return code of {} command", lmstat);
        }
    };
    debug!(
        "flexlm.rs:fetch: external command finished with exit code {}",
        rc
    );

    if !cmd.status.success() {
        bail!("{} command exited with non-normal exit code {}", lmstat, rc);
    }

    let stdout = String::from_utf8(cmd.stdout)?;
    for line in stdout.lines() {
        if let Some(capt) = RE_LMSTAT_USAGE.captures(line) {
            debug!("Line matched: |{}|", line);
            /*
            // NOTE: There is always at least one capture group, the complete string itself
            if capt.len() == 1 {
                continue;
            }
            */

            if capt.len() != 4 {
                error!("Regular expression returns {} capture groups instead of 4", capt.len());
                continue;
            }

            let feature = capt.get(1).map_or("", |m| m.as_str());
            let _total = capt.get(2).map_or("", |m| m.as_str());
            let _used = capt.get(3).map_or("", |m| m.as_str());

            let total: u64 = match _total.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _total, e);
                    continue;
                }
            };

            let used: u64 = match _used.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _used, e);
                    continue;
                }
            };
            debug!("FlexLM license {}, feature {}, total {}, used {}", &lic.name, feature, total, used);
        } else {
            debug!("Line does NOT match: |{}|", line);
        }
    }

    Ok(())
}
