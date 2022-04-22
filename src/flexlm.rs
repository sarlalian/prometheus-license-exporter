use crate::config;
use crate::exporter;
use crate::license;

use lazy_static::lazy_static;
use log::{debug, error};
use prometheus::{IntGaugeVec, Opts};
use regex::Regex;
use simple_error::bail;
use std::collections::HashMap;
use std::error::Error;
use std::process::Command;

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
    pub static ref FLEXLM_FEATURES_USER: IntGaugeVec = IntGaugeVec::new(
        Opts::new("flexlm_feature_used_users", "Number of licenses used by user"),
        &["app", "name", "user"],
    ).unwrap();
    // TODO: Use the same name as FLEXLM_FEATURES_USER (register panics at the moment)
    pub static ref FLEXLM_FEATURES_USER_VERSION: IntGaugeVec = IntGaugeVec::new(
        Opts::new("flexlm_feature_versions_used_users", "Number of licenses and version used by user"),
        &["app", "name", "user", "version"],
    ).unwrap();

}

pub fn fetch(lic: &config::FlexLM, lmutil: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_LMSTAT_USAGE: Regex = Regex::new(r"^Users of ([a-zA-Z0-9_\-+]+):\s+\(Total of (\d+) license[s]? issued;\s+Total of (\d+) license[s]? in use\)$").unwrap();
        static ref RE_LMSTAT_USERS_SINGLE_LICENSE: Regex = Regex::new(r"^\s+(\w+) [\w.\-_]+\s+[\w/]+\s+\(([\w\-.]+)\).*, start [A-Z][a-z][a-z] \d+/\d+ \d+:\d+$").unwrap();
        static ref RE_LMSTAT_USERS_MULTI_LICENSE: Regex = Regex::new(r"^\s+(\w+) [\w.\-_]+\s+[a-zA-Z0-9/]+\s+\(([\w.\-_]+)\)\s+\([\w./\s]+\),\s+start [A-Z][a-z][a-z] \d+/\d+ \d+:\d+,\s+(\d+) licenses$").unwrap();
    }

    // dict -> "feature" -> "user" -> "version" -> count
    let mut fuv: HashMap<String, HashMap<String, HashMap<String, i64>>> = HashMap::new();

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

    let mut feature: &str = "";
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }

        if let Some(capt) = RE_LMSTAT_USAGE.captures(line) {
            if capt.len() != 4 {
                error!(
                    "Regular expression returns {} capture groups instead of 4",
                    capt.len()
                );
                continue;
            }

            feature = capt.get(1).map_or("", |m| m.as_str());
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
        } else if let Some(capt) = RE_LMSTAT_USERS_SINGLE_LICENSE.captures(line) {
            if capt.len() != 3 {
                error!(
                    "Regular expression returns {} capture groups instead of 3",
                    capt.len()
                );
                continue;
            }

            let user = capt.get(1).map_or("", |m| m.as_str());
            let version = capt.get(2).map_or("", |m| m.as_str());

            let feat = fuv
                .entry(feature.to_string())
                .or_insert_with(HashMap::<String, HashMap<String, i64>>::new);
            let usr = feat
                .entry(user.to_string())
                .or_insert_with(HashMap::<String, i64>::new);
            *usr.entry(version.to_string()).or_insert(0) += 1;
        } else if let Some(capt) = RE_LMSTAT_USERS_MULTI_LICENSE.captures(line) {
            if capt.len() != 3 {
                error!(
                    "Regular expression returns {} capture groups instead of 4",
                    capt.len()
                );
                continue;
            }

            let user = capt.get(1).map_or("", |m| m.as_str());
            let version = capt.get(2).map_or("", |m| m.as_str());
            let _count = capt.get(3).map_or("", |m| m.as_str());
            let count: i64 = match _count.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _count, e);
                    continue;
                }
            };

            let feat = fuv
                .entry(feature.to_string())
                .or_insert_with(HashMap::<String, HashMap<String, i64>>::new);
            let usr = feat
                .entry(user.to_string())
                .or_insert_with(HashMap::<String, i64>::new);
            *usr.entry(version.to_string()).or_insert(0) += count;
        }
    }

    let mut users = false;
    let mut versions = false;
    if let Some(export_user) = lic.export_user {
        users = export_user
    }
    if let Some(export_version) = lic.export_version {
        versions = export_version
    }

    if users {
        if versions {
            for (feat, uv) in fuv.iter() {
                for (user, v) in uv.iter() {
                    for (version, count) in v.iter() {
                        debug!(
                            "flexlm.rs:fetch: Setting flexlm_feature_used_users -> {} {} {} {} {}",
                            lic.name, feat, user, version, *count
                        );
                        FLEXLM_FEATURES_USER_VERSION
                            .with_label_values(&[&lic.name, feat, user, version])
                            .set(*count);
                    }
                }
            }
        } else {
            for (feat, uv) in fuv.iter() {
                for (user, v) in uv.iter() {
                    let mut count: i64 = 0;
                    for (_, vcount) in v.iter() {
                        count += *vcount;
                    }
                    debug!(
                        "flexlm.rs:fetch: Setting flexlm_feature_used_users -> {} {} {} {}",
                        lic.name, feat, user, count
                    );
                    FLEXLM_FEATURES_USER
                        .with_label_values(&[&lic.name, feat, user])
                        .set(count);
                }
            }
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
    exporter::REGISTRY
        .register(Box::new(FLEXLM_FEATURES_USER.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(FLEXLM_FEATURES_USER_VERSION.clone()))
        .unwrap();
}
