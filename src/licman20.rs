use crate::config;
use crate::exporter;
use crate::license;

use chrono::NaiveDateTime;
use lazy_static::lazy_static;
use log::{debug, error, warn};
use prometheus::{GaugeVec, IntGaugeVec, Opts};
use regex::Regex;
use simple_error::bail;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::io::Write;
use std::process::{Command, Stdio};

lazy_static! {
}

/*
pub struct FlexLMLicenseExpiration {
    pub feature: String,
    pub version: String,
    pub license_count: i64,
    pub expiration: f64,
    pub vendor: String,
}
*/

pub fn fetch(lic: &config::Licman20, licman20_appl: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_LICMAN20_PRODUCT_KEY: Regex = Regex::new(r"^Product key\s+:\s+(\d+)$").unwrap();
        static ref RE_LICMAN20_TOTAL_LICENSES: Regex = Regex::new(r"^Number of Licenses\s+:\s+(\d+)$").unwrap();
        static ref RE_LICMAN20_USED_LICENSES: Regex = Regex::new(r"^In use\s+:\s+(\d+)$").unwrap();
        static ref RE_LICMAN20_END_DATE: Regex = Regex::new(r"^End date\s+:\s+([\w\-]+)$").unwrap();
        static ref RE_LICMAN20_FEATURE: Regex = Regex::new(r"^Comment\s+:\s+(\w+)$").unwrap();
    }

    // dict -> "feature" -> "user" -> "version" -> count
    let mut fuv: HashMap<String, HashMap<String, HashMap<String, i64>>> = HashMap::new();

    env::set_var("LANG", "C");
    debug!(
        "licman20.rs:fetch: Running {}",
        licman20_appl
    );

    let mut cmd = Command::new(licman20_appl)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    cmd.stdin
        .as_mut()
        .ok_or("Unable to connect to stdin for command")?
        .write_all(b"4\nX\n")?;

    let stdout_and_err = cmd.wait_with_output()?;

    let rc = match stdout_and_err.status.code() {
        Some(v) => v,
        None => {
            bail!("Can't get return code of {} command", licman20_appl);
        }
    };
    debug!(
        "licman20.rs:fetch: external command finished with exit code {}",
        rc
    );

    if !stdout_and_err.status.success() {
        bail!(
            "{} command exited with non-normal exit code {} for {}",
            licman20_appl,
            rc,
            lic.name
        );
    }

    // Note: licman20_appl will print it's result to stderr and only the menu to stdout
    let stderr = String::from_utf8(stdout_and_err.stderr)?;

    let mut feature: &str = "";
    for line in stderr.lines() {
        if line.is_empty() {
            continue;
        }
        debug!("stderr> {}", line);
    }

    Ok(())
}

pub fn register() {}
