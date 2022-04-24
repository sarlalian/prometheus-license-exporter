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
        Opts::new(
            "flexlm_feature_used_users",
            "Number of licenses used by user"
        ),
        &["app", "name", "user", "version"],
    )
    .unwrap();
    pub static ref FLEXLM_SERVER_STATUS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("flexlm_server_status", "Status of license server(s)"),
        &["app", "fqdn", "master", "port", "version"],
    )
    .unwrap();
    pub static ref FLEXLM_VENDOR_STATUS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("flexlm_vendor_status", "Status of the vendor daemon"),
        &["app", "name", "version"],
    )
    .unwrap();
    pub static ref FLEXLM_FEATURE_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "flexlm_feature_expiration_seconds",
            "Time until license features will expire"
        ),
        &["app", "index", "licenses", "name", "vendor", "version"]
    )
    .unwrap();
    pub static ref FLEXLM_FEATURE_AGGREGATED_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "flexlm_feature_aggregate_expiration_seconds",
            "Aggregated licenses by expiration time"
        ),
        &["app", "features", "index", "licenses"]
    )
    .unwrap();
}

pub struct LicenseExpiration {
    pub feature: String,
    pub version: String,
    pub license_count: i64,
    pub expiration: f64,
    pub vendor: String,
}

pub fn fetch(lic: &config::FlexLM, lmutil: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_LMSTAT_USAGE: Regex = Regex::new(r"^Users of ([a-zA-Z0-9_\-+]+):\s+\(Total of (\d+) license[s]? issued;\s+Total of (\d+) license[s]? in use\)$").unwrap();
        static ref RE_LMSTAT_USERS_SINGLE_LICENSE: Regex = Regex::new(r"^\s+(\w+) [\w.\-_]+\s+[\w/]+\s+\(([\w\-.]+)\).*, start [A-Z][a-z][a-z] \d+/\d+ \d+:\d+$").unwrap();
        static ref RE_LMSTAT_USERS_MULTI_LICENSE: Regex = Regex::new(r"^\s+(\w+) [\w.\-_]+\s+[a-zA-Z0-9/]+\s+\(([\w.\-_]+)\)\s+\([\w./\s]+\),\s+start [A-Z][a-z][a-z] \d+/\d+ \d+:\d+,\s+(\d+) licenses$").unwrap();
        static ref RE_LMSTAT_LICENSE_SERVER_STATUS: Regex = Regex::new(r"^License server status:\s+([\w.\-@,]+)$").unwrap();
        static ref RE_LMSTAT_SERVER_STATUS: Regex = Regex::new(r"([\w.\-]+):\s+license server (\w+)\s+(\(MASTER\))?\s*([\w.]+)").unwrap();
        static ref RE_LMSTAT_VENDOR_STATUS: Regex = Regex::new(r"\s+(\w+):\s+(\w+)\s+([\w.]+)$").unwrap();
    }

    // dict -> "feature" -> "user" -> "version" -> count
    let mut fuv: HashMap<String, HashMap<String, HashMap<String, i64>>> = HashMap::new();
    let mut server_port: HashMap<String, String> = HashMap::new();
    let mut server_status: HashMap<String, i64> = HashMap::new();
    let mut server_master: HashMap<String, bool> = HashMap::new();
    let mut server_version: HashMap<String, String> = HashMap::new();
    let mut license_server = String::new();

    env::set_var("LANG", "C");
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
        bail!(
            "{} command exited with non-normal exit code {} for {}",
            lmutil,
            rc,
            lic.name
        );
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
                    capt.len(),
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
            if capt.len() != 4 {
                error!(
                    "Regular expression returns {} capture groups instead of 3",
                    capt.len(),
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
        } else if let Some(capt) = RE_LMSTAT_LICENSE_SERVER_STATUS.captures(line) {
            if capt.len() != 2 {
                error!(
                    "Regular expression returns {} capture groups instead of 2",
                    capt.len()
                );
                continue;
            }
            let status_line = capt.get(1).map_or("", |m| m.as_str());
            license_server = status_line.to_string();

            for server_line in status_line.split(',') {
                let srv_port: Vec<&str> = server_line.split('@').collect();
                server_port.insert(srv_port[1].to_string(), srv_port[0].to_string());
                server_status.insert(srv_port[1].to_string(), 0);
                server_master.insert(srv_port[1].to_string(), false);
                server_version.insert(srv_port[1].to_string(), String::new());
            }
        } else if let Some(capt) = RE_LMSTAT_SERVER_STATUS.captures(line) {
            if capt.len() != 5 {
                error!(
                    "Regular expression returns {} capture groups instead of 5",
                    capt.len()
                );
                continue;
            }
            let server = capt.get(1).map_or("", |m| m.as_str());
            let status = capt.get(2).map_or("", |m| m.as_str());
            let master = capt.get(3).map_or("", |m| m.as_str());
            let version = capt.get(4).map_or("", |m| m.as_str());
            if status == "UP" {
                server_status.insert(server.to_string(), 1);
            }
            if master == "(MASTER)" {
                server_master.insert(server.to_string(), true);
            }
            server_version.insert(server.to_string(), version.to_string());
        } else if let Some(capt) = RE_LMSTAT_VENDOR_STATUS.captures(line) {
            if capt.len() != 4 {
                error!(
                    "Regular expression returns {} capture groups instead of 4",
                    capt.len()
                );
                continue;
            }
            let vendor = capt.get(1).map_or("", |m| m.as_str());
            let _status = capt.get(2).map_or("", |m| m.as_str());
            let mut status: i64 = 0;
            if _status == "UP" {
                status = 1;
            }
            let version = capt.get(3).map_or("", |m| m.as_str());

            debug!(
                "flexlm.rs:fetch: Setting flexlm_vendor_status -> {} {} {} {}",
                lic.name, vendor, version, status
            );
            FLEXLM_VENDOR_STATUS
                .with_label_values(&[&lic.name, vendor, version])
                .set(status);
        }
    }

    if !license_server.is_empty() {
        match fetch_expiration(lic, lmutil, license_server) {
            Ok(_) => {}
            Err(e) => {
                error!("Unable to fetch expiration dates: {}", e);
            }
        };
    } else {
        warn!("No license server informaton received for {}", lic.name);
    }

    for server in server_status.keys() {
        let status = server_status.get(server).unwrap_or(&0);
        let _master = server_master.get(server).unwrap_or(&false);
        let master = format!("{}", _master);
        let port = match server_port.get(server) {
            Some(v) => v,
            None => "",
        };
        let version = match server_version.get(server) {
            Some(v) => v,
            None => "",
        };
        debug!(
            "flexlm.rs:fetch: Setting flexlm_server_status -> {} {} {} {} {} {}",
            lic.name, server, master, port, version, status
        );
        FLEXLM_SERVER_STATUS
            .with_label_values(&[&lic.name, server, &master, port, version])
            .set(*status);
    }

    if lic.export_user.is_some() {
        for (feat, uv) in fuv.iter() {
            for (user, v) in uv.iter() {
                for (version, count) in v.iter() {
                    debug!(
                        "flexlm.rs:fetch: Setting flexlm_feature_used_users -> {} {} {} {} {}",
                        lic.name, feat, user, version, *count
                    );
                    FLEXLM_FEATURES_USER
                        .with_label_values(&[&lic.name, feat, user, version])
                        .set(*count);
                }
            }
        }
    }

    Ok(())
}

fn fetch_expiration(
    lic: &config::FlexLM,
    lmutil: &str,
    license_server: String,
) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_LMSTAT_EXPIRATION: Regex = Regex::new(r"^([\w\-+]+)\s+([\d.]+)\s+(\d+)\s+([\w-]+)\s+(\w+)$").unwrap();
        // Some license servers, especially on MICROS~1 Windows, report Feature,Version,#licenses,Vendor.Expires instead of Feature,Version,#licenses,Expires,Vendor
        static ref RE_LMSTAT_ALTERNATIVE_EXPIRATION: Regex = Regex::new(r"^([\w\-+]+)\s+([\d.]+)\s+(\d+)\s+(\w+)\s+([\w-]+)$").unwrap();
    }

    let mut expiring = Vec::<LicenseExpiration>::new();
    let mut aggregated_expiration: HashMap<String, Vec<LicenseExpiration>> = HashMap::new();
    let mut expiration_dates = Vec::<f64>::new();

    // NOTE: lmutil lmstat -i queries the  local license file. To avoid stale data, we query the extracted
    //       license servers from  lmstat -c ... -a output instead.
    env::set_var("LANG", "C");
    debug!(
        "flexlm.rs:fetch: Running {} -c {} -i",
        lmutil, license_server
    );
    let cmd = Command::new(lmutil)
        .arg("lmstat")
        .arg("-c")
        .arg(license_server)
        .arg("-i")
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
        bail!(
            "{} command exited with non-normal exit code {} for {}",
            lmutil,
            rc,
            lic.name
        );
    }

    let stdout = String::from_utf8(cmd.stdout)?;
    for line in stdout.lines() {
        if let Some(capt) = RE_LMSTAT_EXPIRATION.captures(line) {
            if capt.len() != 6 {
                error!(
                    "Regular expression returns {} capture groups instead of 6",
                    capt.len()
                );
                continue;
            }
            let feature = capt.get(1).map_or("", |m| m.as_str());
            let version = capt.get(2).map_or("", |m| m.as_str());
            let _count = capt.get(3).map_or("", |m| m.as_str());
            let count: i64 = match _count.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _count, e);
                    continue;
                }
            };

            let _expiration = capt.get(4).map_or("", |m| m.as_str());
            let expiration: f64;
            if _expiration == "1-jan-0"
                || _expiration == "01-jan-0000"
                || _expiration.starts_with("permanent")
            {
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

            let vendor = capt.get(5).map_or("", |m| m.as_str());

            expiration_dates.push(expiration);
            expiring.push(LicenseExpiration {
                feature: feature.to_string(),
                version: version.to_string(),
                license_count: count,
                expiration,
                vendor: vendor.to_string(),
            });

            let expiration_str = expiration.to_string();
            let aggregated = aggregated_expiration
                .entry(expiration_str)
                .or_insert_with(Vec::<LicenseExpiration>::new);
            aggregated.push(LicenseExpiration {
                feature: feature.to_string(),
                version: version.to_string(),
                license_count: count,
                expiration,
                vendor: vendor.to_string(),
            });
        } else if let Some(capt) = RE_LMSTAT_ALTERNATIVE_EXPIRATION.captures(line) {
            if capt.len() != 6 {
                error!(
                    "Regular expression returns {} capture groups instead of 6",
                    capt.len()
                );
                continue;
            }
            let feature = capt.get(1).map_or("", |m| m.as_str());
            let version = capt.get(2).map_or("", |m| m.as_str());
            let _count = capt.get(3).map_or("", |m| m.as_str());
            let count: i64 = match _count.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as interger: {}", _count, e);
                    continue;
                }
            };

            let _expiration = capt.get(5).map_or("", |m| m.as_str());
            let expiration: f64;
            if _expiration == "1-jan-0"
                || _expiration == "01-jan-0000"
                || _expiration.starts_with("permanent")
            {
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
            let vendor = capt.get(4).map_or("", |m| m.as_str());
            expiring.push(LicenseExpiration {
                feature: feature.to_string(),
                version: version.to_string(),
                license_count: count,
                expiration,
                vendor: vendor.to_string(),
            });

            let expiration_str = expiration.to_string();
            let aggregated = aggregated_expiration
                .entry(expiration_str)
                .or_insert_with(Vec::<LicenseExpiration>::new);
            aggregated.push(LicenseExpiration {
                feature: feature.to_string(),
                version: version.to_string(),
                license_count: count,
                expiration,
                vendor: vendor.to_string(),
            });
        }
    }

    let mut index: i64 = 1;
    for entry in expiring {
        debug!(
            "flexlm.rs:fetch: Setting flexlm_feature_used_users -> {} {} {} {} {} {} {}",
            lic.name,
            index,
            entry.license_count.to_string(),
            entry.feature,
            entry.vendor,
            entry.version,
            entry.expiration
        );
        FLEXLM_FEATURE_EXPIRATION
            .with_label_values(&[
                &lic.name,
                &index.to_string(),
                &entry.license_count.to_string(),
                &entry.feature,
                &entry.vendor,
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
                license_count += entry.license_count;
                feature_count += 1;
            }
            debug!("flexlm.rs:fetch_expiration: Setting flexlm_feature_aggregate_expiration_seconds -> {} {} {} {} {}", lic.name, feature_count, index, license_count, exp);
            FLEXLM_FEATURE_AGGREGATED_EXPIRATION
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
        .register(Box::new(FLEXLM_SERVER_STATUS.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(FLEXLM_VENDOR_STATUS.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(FLEXLM_FEATURE_EXPIRATION.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(FLEXLM_FEATURE_AGGREGATED_EXPIRATION.clone()))
        .unwrap();
}
