use crate::config;
use crate::exporter;
use crate::license;

use lazy_static::lazy_static;
use log::{debug, error};
use prometheus::{IntGaugeVec, Opts};
use regex::Regex;
use simple_error::bail;
use std::error::Error;
use std::process::Command;

// FlexLM - be comptatible to https://github.com/mjtrangoni/flexlm_exporter
lazy_static! {
    pub static ref FLEXLM_FEATURES_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("flexlm_feature_issued", "Total number of issued licenses"),
        &["app", "name"],
    )
    .unwrap();
    pub static ref FLEXLM_FEATURES_USED: IntGaugeVec = IntGaugeVec::new(
        Opts::new("flexlm_feature_used", "Number of used licenses"),
        &["app", "name"],
    )
    .unwrap();
}

pub fn fetch(lic: &config::FlexLM, lmutil: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_LMSTAT_USAGE: Regex = Regex::new(r"^Users of ([a-zA-Z0-9_\-+]+):\s+\(Total of (\d+) license[s]? issued;\s+Total of (\d+) license[s]? in use\)$").unwrap();
        static ref RE_LMSTAT_USERS_SINGLE_LICENSE: Regex = Regex::new(r"^\s+(\w+) [\w.\-_]+\s+[\w/]+\s+\(([\w\-.]+)\).*, start [A-Z][a-z][a-z] \d+/\d+ \d+:\d+$").unwrap();
        static ref RE_LMSTAT_USERS_MULTI_LICENSE: Regex = Regex::new(r"^\s+(\w+) [\w.\-_]+\s+[a-zA-Z0-9/]+\s+\(([\w.\-_]+)\)\s+\([\w./\s]+\),\s+start [A-Z][a-z][a-z] \d+/\d+ \d+:\d+,\s+(\d+) licenses$").unwrap();
    }

    debug!("flexlm.rs:fetch: Running {} -c {} -a", lmutil, &lic.license);
    let cmd = Command::new(lmutil)
        .arg("lmstat")
        .arg("-c")
        .arg(&lic.license)
        .arg("-a")
        .output()?;

    let rc = match cmd.status.code() {
        Some(v) => v,
        None => {
            bail!("Can't get return code of {} command", lmutil);
        }
    };
    debug!(
        "flexlm.rs:fetch: external command finished with exit code {}",
        rc
    );

    if !cmd.status.success() {
        bail!("{} command exited with non-normal exit code {}", lmutil, rc);
    }

    let stdout = String::from_utf8(cmd.stdout)?;
    for line in stdout.lines() {
        if let Some(capt) = RE_LMSTAT_USAGE.captures(line) {
            if capt.len() != 4 {
                error!(
                    "Regular expression returns {} capture groups instead of 4",
                    capt.len()
                );
                continue;
            }

            let feature = capt.get(1).map_or("", |m| m.as_str());
            let _total = capt.get(2).map_or("", |m| m.as_str());
            let _used = capt.get(3).map_or("", |m| m.as_str());

            if license::is_excluded(&lic.excluded_features, feature.to_string()) {
                debug!("flexlm.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", feature, lic.name);
                continue;
            }

            let total: i64 = match _total.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _total, e);
                    continue;
                }
            };

            let used: i64 = match _used.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _used, e);
                    continue;
                }
            };

            debug!(
                "flexlm.rs:fetch: Setting flexlm_feature_issued -> {} {} {}",
                lic.name, feature, total
            );
            FLEXLM_FEATURES_TOTAL
                .with_label_values(&[&lic.name, feature])
                .set(total);

            debug!(
                "flexlm.rs:fetch: Setting flexlm_feature_used -> {} {} {}",
                lic.name, feature, used
            );
            FLEXLM_FEATURES_USED
                .with_label_values(&[&lic.name, feature])
                .set(used);
        }
    }

    Ok(())
}

pub fn register() {
    exporter::REGISTRY
        .register(Box::new(FLEXLM_FEATURES_TOTAL.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(FLEXLM_FEATURES_USED.clone()))
        .unwrap();
}
