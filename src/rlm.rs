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
use std::process::Command;

lazy_static! {
    pub static ref RLM_FEATURES_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rlm_feature_issued", "Total number of issued licenses"),
        &["app", "name", "version"],
    )
    .unwrap();
    pub static ref RLM_FEATURES_USED: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rlm_feature_used", "Number of used licenses"),
        &["app", "name", "version"],
    )
    .unwrap();
    pub static ref RLM_FEATURES_USER: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rlm_feature_used_users", "Number of licenses used by user"),
        &["app", "name", "user", "version"],
    )
    .unwrap();
    pub static ref RLM_FEATURE_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "rlm_feature_expiration_seconds",
            "Time until license features will expire"
        ),
        &["app", "index", "licenses", "name", "version"]
    )
    .unwrap();
    pub static ref RLM_FEATURE_AGGREGATED_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "rlm_feature_aggregate_expiration_seconds",
            "Aggregated licenses by expiration time"
        ),
        &["app", "features", "index", "licenses"]
    )
    .unwrap();
    pub static ref RLM_SERVER_STATUS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("rlm_server_status", "Status of license server(s)"),
        &["app", "fqdn", "port", "version"],
    )
    .unwrap();
}

pub struct LicenseData {
    pub feature: String,
    pub version: String,
    pub expiration: f64,
    pub total: i64,
    pub reserved: i64,
    pub used: i64,
}

pub fn fetch(lic: &config::Rlm, rlmutil: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_RLM_FEATURE_VERSION: Regex =
            Regex::new(r"^\s+([\w\-.]+)\s([\w.]+)$").unwrap();
        static ref RE_RLM_USAGE: Regex = Regex::new(
            r"^\s+count:\s+(\d+),\s+# reservations:\s+(\d+),\s+inuse:\s+(\d+), exp:\s+([\w\-]+)"
        )
        .unwrap();
    }

    // feature -> version = usage
    let mut fv: HashMap<String, HashMap<String, HashMap<String, LicenseData>>> = HashMap::new();
    let mut expiring = Vec::<LicenseData>::new();
    let mut aggregated_expiration: HashMap<String, Vec<LicenseData>> = HashMap::new();
    let mut expiration_dates = Vec::<f64>::new();

    env::set_var("LANG", "C");
    debug!(
        "rlm.rs:fetch: Running {} rlmstat -c {} -l {}",
        rlmutil, &lic.license, &lic.isv
    );
    let cmd = Command::new(rlmutil)
        .arg("rlmstat")
        .arg("-c")
        .arg(&lic.license)
        .arg("-l")
        .arg(&lic.isv)
        .output()?;

    let rc = match cmd.status.code() {
        Some(v) => v,
        None => {
            bail!("Can't get return code of {} command", rlmutil);
        }
    };
    debug!(
        "rlm.rs:fetch: external command finished with exit code {}",
        rc
    );

    if !cmd.status.success() {
        bail!(
            "{} command exited with non-normal exit code {} for {}",
            rlmutil,
            rc,
            lic.name
        );
    }

    let stdout = String::from_utf8(cmd.stdout)?;

    let mut feature: &str = "";
    let mut version: &str = "";
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }

        if let Some(capt) = RE_RLM_FEATURE_VERSION.captures(line) {
            if capt.len() != 3 {
                error!(
                    "Regular expression returns {} capture groups instead of 4",
                    capt.len()
                );
                continue;
            }

            debug!("rlm.rs:fetch: RE_RLM_FEATURE_VERSION match on '{}'", line);

            feature = capt.get(1).map_or("", |m| m.as_str());
            version = capt.get(2).map_or("", |m| m.as_str());

            if license::is_excluded(&lic.excluded_features, feature.to_string()) {
                debug!("flexlm.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", feature, lic.name);
                feature = "";
                continue;
            }
        } else if let Some(capt) = RE_RLM_USAGE.captures(line) {
            // NOTE: An empty value for feature indicates a skipped feature from the exclusion list
            if feature.is_empty() {
                continue;
            }

            if capt.len() != 5 {
                error!(
                    "Regular expression returns {} capture groups instead of 5",
                    capt.len()
                );
                continue;
            }

            debug!("rlm.rs:fetch: RE_RLM_USAGE match on '{}'", line);

            let _total = capt.get(1).map_or("", |m| m.as_str());
            let total: i64 = match _total.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _total, e);
                    continue;
                }
            };

            let _reserved = capt.get(2).map_or("", |m| m.as_str());
            let reserved: i64 = match _reserved.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _reserved, e);
                    continue;
                }
            };

            let _used = capt.get(3).map_or("", |m| m.as_str());
            let used: i64 = match _used.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _used, e);
                    continue;
                }
            };

            let _expiration = capt.get(4).map_or("", |m| m.as_str());
            let expiration: f64;

            if _expiration == "permanent" {
                expiration = f64::INFINITY;
            } else {
                expiration = match NaiveDateTime::parse_from_str(
                    &format!("{} 00:00:00", _expiration),
                    "%d-%b-%Y %H:%M:%S",
                ) {
                    Ok(v) => v.timestamp() as f64,
                    Err(e) => {
                        error!("Can't parse {} as date and time: {}", _expiration, e);
                        continue;
                    }
                };
            }

            expiration_dates.push(expiration);
            expiring.push(LicenseData {
                feature: feature.to_string(),
                version: version.to_string(),
                expiration,
                total,
                reserved,
                used,
            });

            let expiration_str = expiration.to_string();
            let aggregated = aggregated_expiration
                .entry(expiration_str)
                .or_insert_with(Vec::<LicenseData>::new);
            aggregated.push(LicenseData {
                feature: feature.to_string(),
                version: version.to_string(),
                expiration,
                total,
                reserved,
                used,
            });

            let feat = fv
                .entry(feature.to_string())
                .or_insert_with(HashMap::<String, HashMap<String, LicenseData>>::new);
            let ver = feat
                .entry(feature.to_string())
                .or_insert_with(HashMap::<String, LicenseData>::new);

            ver.insert(
                version.to_string(),
                LicenseData {
                    feature: feature.to_string(),
                    version: version.to_string(),
                    expiration,
                    total,
                    reserved,
                    used,
                },
            );

            debug!(
                "rlm.rs:fetch: Setting rlm_feature_issued {} {} {} -> {}",
                lic.name, feature, version, total
            );
            RLM_FEATURES_TOTAL
                .with_label_values(&[&lic.name, feature, version])
                .set(total);

            debug!(
                "rlm.rs:fetch: Setting rlm_feature_used {} {} {} -> {}",
                lic.name, feature, version, used
            );
            RLM_FEATURES_USED
                .with_label_values(&[&lic.name, feature, version])
                .set(used);
        } else {
            debug!("rlm.rs:fetch: No regexp matches '{}'", line);
        }
    }

    if let Some(report_users) = lic.export_user {
        if report_users {
            match fetch_checkouts(lic, rlmutil) {
                Ok(_) => {}
                Err(e) => {
                    error!("Unable to fetch license checkouts: {}", e);
                }
            };
        }
    }

    match fetch_status(lic, rlmutil) {
        Ok(_) => {}
        Err(e) => {
            error!("Unable to fetch server status: {}", e);
        }
    };

    let mut index: i64 = 1;
    for entry in expiring {
        if license::is_excluded(&lic.excluded_features, entry.feature.to_string()) {
            debug!(
                "rlm.rs:fetch: Skipping feature {} because it is in excluded_features list of {}",
                entry.feature, lic.name
            );
            continue;
        }

        debug!(
            "rlm.rs:fetch: Setting rlm_feature_used_users -> {} {} {} {} {} {}",
            lic.name,
            index,
            entry.total.to_string(),
            entry.feature,
            entry.version,
            entry.expiration
        );
        RLM_FEATURE_EXPIRATION
            .with_label_values(&[
                &lic.name,
                &index.to_string(),
                &entry.total.to_string(),
                &entry.feature,
                &entry.version,
            ])
            .set(entry.expiration);
        index += 1;
    }

    index = 0;

    expiration_dates.sort_by(|a, b| a.partial_cmp(b).unwrap());
    expiration_dates.dedup_by(|a, b| a == b);

    for exp in expiration_dates {
        let exp_str = exp.to_string();
        if let Some(v) = aggregated_expiration.get(&exp_str) {
            let mut license_count: i64 = 0;
            let mut feature_count: i64 = 0;
            for entry in v {
                license_count += entry.total;
                feature_count += 1;
            }
            debug!("rlm.rs:fetch_expiration: Setting rlm_feature_aggregate_expiration_seconds -> {} {} {} {} {}", lic.name, feature_count, index, license_count, exp);
            RLM_FEATURE_AGGREGATED_EXPIRATION
                .with_label_values(&[
                    &lic.name,
                    &feature_count.to_string(),
                    &index.to_string(),
                    &license_count.to_string(),
                ])
                .set(exp);
            index += 1;
        } else {
            warn!("Key {} not found in HashMap aggregated", exp_str);
        }
    }

    Ok(())
}

fn fetch_checkouts(lic: &config::Rlm, rlmutil: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_RLM_CHECKOUTS: Regex = Regex::new(r"^\s+([\w\-.]+)\s+([\w.]+):\s+([\w\-.@]+)\s+\d+/\d+\s+at\s+\d+/\d+\s+\d+:\d+\s+\(handle:\s+\w+\)$").unwrap();
    }
    // dict -> "feature" -> "user" -> "version" -> count
    let mut fuv: HashMap<String, HashMap<String, HashMap<String, i64>>> = HashMap::new();

    env::set_var("LANG", "C");
    debug!(
        "rlm.rs:fetch: Running {} rlmstat -c {} -i {}",
        rlmutil, &lic.license, &lic.isv
    );
    let cmd = Command::new(rlmutil)
        .arg("rlmstat")
        .arg("-c")
        .arg(&lic.license)
        .arg("-i")
        .arg(&lic.isv)
        .output()?;

    let rc = match cmd.status.code() {
        Some(v) => v,
        None => {
            bail!("Can't get return code of {} command", rlmutil);
        }
    };
    debug!(
        "rlm.rs:fetch: external command finished with exit code {}",
        rc
    );

    if !cmd.status.success() {
        bail!(
            "{} command exited with non-normal exit code {} for {}",
            rlmutil,
            rc,
            lic.name
        );
    }

    let stdout = String::from_utf8(cmd.stdout)?;

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }

        if let Some(capt) = RE_RLM_CHECKOUTS.captures(line) {
            if capt.len() != 4 {
                error!(
                    "Regular expression returns {} capture groups instead of 4",
                    capt.len(),
                );
                continue;
            }

            debug!(
                "rlm.rs:fetch_checkouts: RE_RLM_FEATURE_VERSION match on '{}'",
                line
            );

            let feature = capt.get(1).map_or("", |m| m.as_str());
            let version = capt.get(2).map_or("", |m| m.as_str());
            let _user: Vec<&str> = capt.get(3).map_or("", |m| m.as_str()).split('@').collect();
            let user = _user[0];

            let feat = fuv
                .entry(feature.to_string())
                .or_insert_with(HashMap::<String, HashMap<String, i64>>::new);
            let usr = feat
                .entry(user.to_string())
                .or_insert_with(HashMap::<String, i64>::new);
            *usr.entry(version.to_string()).or_insert(0) += 1;
        } else {
            debug!("rlm.rs:fetch_checkouts: No regexp matches '{}'", line);
        }
    }

    for (feat, uv) in fuv.iter() {
        for (user, v) in uv.iter() {
            for (version, count) in v.iter() {
                if license::is_excluded(&lic.excluded_features, feat.to_string()) {
                    debug!("rlm.rs:fetch_checkouts: Skipping feature {} because it is in excluded_features list of {}", feat, lic.name);
                    continue;
                }
                debug!(
                    "rlm.rs:fetch_checkouts: Setting rlm_feature_used_users -> {} {} {} {} {}",
                    lic.name, feat, user, version, *count
                );
                RLM_FEATURES_USER
                    .with_label_values(&[&lic.name, feat, user, version])
                    .set(*count);
            }
        }
    }

    Ok(())
}

fn fetch_status(lic: &config::Rlm, rlmutil: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_RLM_STATUS: Regex =
            Regex::new(r"^\s+[\w+\-.]+ ISV server status on [\w\-.]+ \(port (\d+)\), (\w+).*$")
                .unwrap();
        static ref RE_RLM_VERSION: Regex =
            Regex::new(r"^\s+[\w+\-.]+ software version ([\w\s.:\-()]+)$").unwrap();
    }

    for server in lic.license.split(':') {
        env::set_var("LANG", "C");
        debug!(
            "rlm.rs:fetch_statush: Running {} rlmstat -c {} -l {}",
            rlmutil, &lic.license, &lic.isv
        );
        let cmd = Command::new(rlmutil)
            .arg("rlmstat")
            .arg("-c")
            .arg(server)
            .arg("-l")
            .arg(&lic.isv)
            .output()?;

        let rc = match cmd.status.code() {
            Some(v) => v,
            None => {
                bail!("Can't get return code of {} command", rlmutil);
            }
        };
        debug!(
            "rlm.rs:fetch_status: external command finished with exit code {}",
            rc
        );

        if !cmd.status.success() {
            bail!(
                "{} command exited with non-normal exit code {} for {}",
                rlmutil,
                rc,
                lic.name
            );
        }

        let stdout = String::from_utf8(cmd.stdout)?;
        let mut port: &str = "";
        let mut status: i64 = 0;
        let mut version: &str = "";
        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }

            if let Some(capt) = RE_RLM_STATUS.captures(line) {
                if capt.len() != 3 {
                    error!(
                        "Regular expression returns {} capture groups instead of 3",
                        capt.len(),
                    );
                    continue;
                }

                debug!("rlm.rs:fetch_status: RE_RLM_STATUS match on '{}'", line);

                port = capt.get(1).map_or("", |m| m.as_str());
                let _status = capt.get(2).map_or("", |m| m.as_str());
                if _status.to_lowercase() == "up" {
                    status = 1;
                }
            } else if let Some(capt) = RE_RLM_VERSION.captures(line) {
                if capt.len() != 3 {
                    error!(
                        "Regular expression returns {} capture groups instead of 3",
                        capt.len(),
                    );
                    continue;
                }

                debug!("rlm.rs:fetch_status: RE_RLM_VERSION match on '{}'", line);

                version = capt.get(1).map_or("", |m| m.as_str());
            } else {
                debug!("rlm.rs:fetch_status: No regexp matches '{}'", line);
            }
        }

        debug!(
            "rlm.rs:fetch_status: Setting rlm_server_status {} {} {} {} -> {}",
            lic.name, server, port, version, status
        );
        RLM_SERVER_STATUS
            .with_label_values(&[&lic.name, server, port, version])
            .set(status);
    }

    Ok(())
}

pub fn register() {
    exporter::REGISTRY
        .register(Box::new(RLM_FEATURES_TOTAL.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(RLM_FEATURES_USED.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(RLM_FEATURES_USER.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(RLM_FEATURE_EXPIRATION.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(RLM_FEATURE_AGGREGATED_EXPIRATION.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(RLM_SERVER_STATUS.clone()))
        .unwrap();
}
