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
    pub static ref DSLS_FEATURES_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("dsls_feature_issued", "Total number of issued licenses"),
        &["app", "name"],
    )
    .unwrap();
    pub static ref DSLS_FEATURES_USED: IntGaugeVec = IntGaugeVec::new(
        Opts::new("dsls_feature_used", "Number of used licenses"),
        &["app", "name"],
    )
    .unwrap();
    pub static ref DSLS_FEATURES_USER: IntGaugeVec = IntGaugeVec::new(
        Opts::new("dsls_feature_used_users", "Number of licenses used by user"),
        &["app", "name", "user"],
    )
    .unwrap();
    pub static ref DSLS_SERVER_STATUS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("dsls_server_status", "Status of license server(s)"),
        &["app", "fqdn", "port", "version"],
    )
    .unwrap();
    pub static ref DSLS_FEATURE_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "dsls_feature_expiration_seconds",
            "Time until license features will expire"
        ),
        &["app", "index", "licenses", "name"]
    )
    .unwrap();
    pub static ref DSLS_FEATURE_AGGREGATED_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "dsls_feature_aggregate_expiration_seconds",
            "Aggregated licenses by expiration time"
        ),
        &["app", "features", "index", "licenses"]
    )
    .unwrap();
}

#[derive(Clone, Debug)]
struct DslsLicenseUsage {
    pub feature: String,
    pub count: i64,
    pub inuse: i64,
    pub user: Option<String>,
}

struct DslsLicenseExpiration {
    pub feature: String,
    pub license_count: i64,
    pub expiration: f64,
}

pub fn fetch(lic: &config::Dsls, dslicsrv: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_DSLS_VERSION: Regex =
            Regex::new(r"^\s+Software version:\s+([\d.\-]+)$").unwrap();
        static ref RE_DSLS_STATUS: Regex = Regex::new(r"^\s+Ready:\s+(\w+).*$").unwrap();
    }

    // dict -> "feature" -> "user" -> count
    let mut fuv: HashMap<String, HashMap<String, i64>> = HashMap::new();
    let mut f_total: HashMap<String, i64> = HashMap::new();
    let mut f_used: HashMap<String, i64> = HashMap::new();
    let mut server_port: HashMap<String, String> = HashMap::new();
    let mut server_version: HashMap<String, String> = HashMap::new();
    let mut server_status: HashMap<String, i64> = HashMap::new();
    let mut license_data: Vec<DslsLicenseUsage> = Vec::new();

    for (_, lserver) in lic.license.split(':').enumerate() {
        let srvport: Vec<&str> = lserver.split('@').collect();

        // NOTE: Configuration validation checks for valid server lines
        let port = srvport[0].to_string();
        let srv = srvport[1].to_string();

        server_port.insert(srv, port);
    }

    let mut features_exported = false;
    let mut csv_mode = false;

    for (server, port) in &server_port {
        env::set_var("LANG", "C");
        debug!(
            "dsls.rs:fetch: Running {} -admin -run \"connect {} {};getLicenseUsage -csv;quit;\"",
            dslicsrv, server, port
        );
        let cmd = Command::new(dslicsrv)
            .arg("-admin")
            .arg("-run")
            .arg(format!(
                "connect {} {};getLicenseUsage -csv;quit;",
                server, port
            ))
            .output()?;

        let rc = match cmd.status.code() {
            Some(v) => v,
            None => {
                bail!("Can't get return code of {} command", dslicsrv);
            }
        };
        debug!(
            "dsls.rs:fetch: external command finished with exit code {}",
            rc
        );

        if !cmd.status.success() {
            bail!(
                "{} command exited with non-normal exit code {} for {}",
                dslicsrv,
                rc,
                lic.name
            );
        }

        let stdout = String::from_utf8(cmd.stdout)?;
        for line in stdout.lines() {
            if let Some(capt) = RE_DSLS_VERSION.captures(line) {
                if capt.len() != 2 {
                    error!(
                        "dsls.rs:fetch: Regular expression returns {} capture groups instead of 2 for RE_DSLS_VERSION",
                        capt.len()
                    );
                    continue;
                }

                debug!("dsls.rs:fetch: RE_DSLS_VERSION match on '{}'", line);
                let version = capt.get(1).map_or("", |m| m.as_str());
                server_version.insert(server.clone(), version.to_string());
            } else if let Some(capt) = RE_DSLS_STATUS.captures(line) {
                if capt.len() != 2 {
                    error!(
                        "dsls.rs:fetch: Regular expression returns {} capture groups instead of 2 for RE_DSLS_STATUS",
                        capt.len()
                    );
                    continue;
                }

                debug!("dsls.rs:fetch: RE_DSLS_STATUS match on '{}'", line);
                let _status = capt.get(1).map_or("", |m| m.as_str());
                let status: i64 = match _status {
                    "yes" => 1,
                    _ => 0,
                };
                server_status.insert(server.clone(), status);
                if features_exported {
                    debug!(
                        "dsls.rs:fetch: Features were already exported, skipping for server {}",
                        server
                    );
                    break;
                }
            } else if line == "admin >getLicenseUsage -csv" {
                debug!("dsls.rs:fetch: enabling CSV mode");
                csv_mode = true;
            } else if line == "admin >quit" {
                debug!("dsls.rs:fetch: setting features_exported to true");
                features_exported = true;

                debug!("dsls.rs:fetch: disabling CSV mode");
                csv_mode = false;
            } else if csv_mode {
                if line.starts_with("Editor,") {
                    continue;
                }
                let data = extract_data(line)?;
                debug!("dsls.rs:fetch: license data: {:?}", data);
                license_data.push(data);
            } else {
                debug!("dsls.rs:fetch: No match on '{}'", line);
            }
        }
    }

    for (server, port) in &server_port {
        if let Some(status) = server_status.get(server) {
            if *status == 1 {
                match fetch_expiration(lic, dslicsrv, server, port) {
                    Ok(_) => {
                        break;
                    }
                    Err(e) => {
                        error!("dsls.rs:fetch: Unable to fetch expiration dates: {}", e);
                    }
                };
            }
        }
    }

    for l in license_data {
        if license::is_excluded(&lic.excluded_features, l.feature.to_string()) {
            debug!(
                "dsls.rs:fetch: Skipping feature {} because it is in excluded_features list of {}",
                l.feature, lic.name
            );
            continue;
        }

        f_used.entry(l.feature.clone()).or_insert(l.inuse);
        f_total.entry(l.feature.clone()).or_insert(l.count);

        if let Some(user) = l.user {
            let feat = fuv
                .entry(l.feature.to_string())
                .or_insert_with(HashMap::<String, i64>::new);
            *feat.entry(user.to_string()).or_insert(0) += l.count;
        }
    }

    for l in f_used.keys() {
        if let Some(used) = f_used.get(l) {
            debug!(
                "dsls.rs:fetch: Setting dsls_feature_used {} {} -> {}",
                lic.name, l, used
            );
            DSLS_FEATURES_USED
                .with_label_values(&[&lic.name, l])
                .set(*used);
        }
        if let Some(total) = f_total.get(l) {
            debug!(
                "dsls.rs:fetch: Setting dsls_feature_issued {} {} -> {}",
                lic.name, l, total
            );
            DSLS_FEATURES_TOTAL
                .with_label_values(&[&lic.name, l])
                .set(*total);
        }
    }

    for (k, v) in &server_status {
        if let Some(port) = server_port.get(k) {
            if let Some(ver) = server_version.get(k) {
                debug!(
                    "dsls.rs:fetch: Setting dsls_server_status {} {} {} {} -> {}",
                    lic.name, k, port, ver, v
                );
                DSLS_SERVER_STATUS
                    .with_label_values(&[&lic.name, k, port, ver])
                    .set(*v);
            }
        }
    }

    if let Some(export_user) = lic.export_user {
        if export_user {
            for (feat, uv) in fuv.iter() {
                for (user, count) in uv.iter() {
                    if license::is_excluded(&lic.excluded_features, feat.to_string()) {
                        debug!("dsls.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", feat, lic.name);
                        continue;
                    }
                    debug!(
                        "dsls.rs:fetch: Setting dsls_feature_used_users {} {} {} -> {}",
                        lic.name, feat, user, *count
                    );
                    DSLS_FEATURES_USER
                        .with_label_values(&[&lic.name, feat, user])
                        .set(*count);
                }
            }
        }
    }

    Ok(())
}

fn extract_data(line: &str) -> Result<DslsLicenseUsage, Box<dyn Error>> {
    // Format is:
    // 0      1        2       3     4               5                  6                7                 8                   9               10          11    12    13     14                15   16 ...
    // Editor,EditorId,Feature,Model,Commercial Type,Max Release Number,Max Release Date,Pricing Structure,Max Casual Duration,Expiration Date,Customer ID,Count,Inuse,Tokens,Casual Usage (mn),Host,User,Internal ID,Active Process,Client Code Version,Session ID,Granted Since,Last Used At,Granted At,Queue Position,

    let splitted: Vec<&str> = line.split(',').collect();
    if splitted.len() < 13 {
        bail!(
            "Invalid DSLS license usage data - expected at least 13 fields but got {} instead",
            splitted.len()
        );
    }

    let feature = splitted[2].to_string();

    let count: i64 = splitted[11].parse()?;
    let inuse: i64 = splitted[12].parse()?;
    let user: Option<String> = if splitted.len() < 17 {
        None
    } else {
        Some(splitted[16].to_string())
    };

    Ok(DslsLicenseUsage {
        feature,
        count,
        inuse,
        user,
    })
}

fn fetch_expiration(
    lic: &config::Dsls,
    dslicsrv: &str,
    server: &str,
    port: &str,
) -> Result<(), Box<dyn Error>> {
    let mut expiring = Vec::<DslsLicenseExpiration>::new();
    let mut aggregated_expiration: HashMap<String, Vec<DslsLicenseExpiration>> = HashMap::new();
    let mut expiration_dates = Vec::<f64>::new();

    env::set_var("LANG", "C");
    debug!(
        "dsls.rs:fetch_expiration: Running {} -admin -run \"connect {} {};getLicenseUsage -short -csv;quit;\"",
        dslicsrv, server, port
    );
    let cmd = Command::new(dslicsrv)
        .arg("-admin")
        .arg("-run")
        .arg(format!(
            "connect {} {};getLicenseUsage -short -csv;quit;",
            server, port
        ))
        .output()?;

    let rc = match cmd.status.code() {
        Some(v) => v,
        None => {
            bail!("Can't get return code of {} command", dslicsrv);
        }
    };
    debug!(
        "dsls.rs:fetch_expiration: external command finished with exit code {}",
        rc
    );

    if !cmd.status.success() {
        bail!(
            "{} command exited with non-normal exit code {} for {}",
            dslicsrv,
            rc,
            lic.name
        );
    }

    let stdout = String::from_utf8(cmd.stdout)?;
    let mut csv_mode = false;

    for line in stdout.lines() {
        // Format of the short CSV output is
        //
        // 0      1        2       3     4               5                  6                7                 8                   9               10          11    12
        // Editor,EditorId,Feature,Model,Commercial Type,Max Release Number,Max Release Date,Pricing Structure,Max Casual Duration,Expiration Date,Customer ID,Count,Inuse,
        if line.starts_with("Editor,") {
            csv_mode = true
        } else if csv_mode {
            let splitted: Vec<&str> = line.split(',').collect();
            if splitted.len() >= 12 {
                let feature = splitted[2].to_string();
                let expiration_date = splitted[9];

                let expiration =
                    match NaiveDateTime::parse_from_str(expiration_date, "%Y-%m-%d %H:%M:%S") {
                        Ok(v) => v.timestamp() as f64,
                        Err(e) => {
                            bail!("Can't parse {} as date and time: {}", expiration_date, e);
                        }
                    };

                let lcount: i64 = match splitted[11].parse() {
                    Ok(v) => v,
                    Err(e) => {
                        error!(
                            "dsls.rs:fetch_expiration: Can't parse {} as integer: {}",
                            splitted[11], e
                        );
                        continue;
                    }
                };

                expiration_dates.push(expiration);
                expiring.push(DslsLicenseExpiration {
                    feature: feature.to_string(),
                    license_count: lcount,
                    expiration,
                });

                let expiration_str = expiration.to_string();
                let aggregated = aggregated_expiration
                    .entry(expiration_str)
                    .or_insert_with(Vec::<DslsLicenseExpiration>::new);
                aggregated.push(DslsLicenseExpiration {
                    feature: feature.to_string(),
                    license_count: lcount,
                    expiration,
                });
            }
        }
    }

    let mut index: i64 = 1;
    for entry in expiring {
        if license::is_excluded(&lic.excluded_features, entry.feature.to_string()) {
            debug!("dsls.rs:fetch_expiration: Skipping feature {} because it is in excluded_features list of {}", entry.feature, lic.name);
            continue;
        }

        debug!(
            "dsls.rs:fetch_expiration: Setting dsls_feature_used_users {} {} {} {} -> {}",
            lic.name,
            index,
            entry.license_count.to_string(),
            entry.feature,
            entry.expiration
        );
        DSLS_FEATURE_EXPIRATION
            .with_label_values(&[
                &lic.name,
                &index.to_string(),
                &entry.license_count.to_string(),
                &entry.feature,
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
            debug!("dsls.rs:fetch_expiration: Setting dsls_feature_aggregate_expiration_seconds {} {} {} {} -> {}", lic.name, feature_count, index, license_count, exp);
            DSLS_FEATURE_AGGREGATED_EXPIRATION
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
        .register(Box::new(DSLS_FEATURES_TOTAL.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(DSLS_FEATURES_USED.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(DSLS_FEATURES_USER.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(DSLS_SERVER_STATUS.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(DSLS_FEATURE_EXPIRATION.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(DSLS_FEATURE_AGGREGATED_EXPIRATION.clone()))
        .unwrap();
}
