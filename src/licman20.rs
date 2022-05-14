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
    pub static ref LICMAN20_FEATURES_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("licman20_feature_issued", "Total number of issued licenses"),
        &["app", "name", "product_key"],
    )
    .unwrap();
    pub static ref LICMAN20_FEATURES_USED: IntGaugeVec = IntGaugeVec::new(
        Opts::new("licman20_feature_used", "Number of used licenses"),
        &["app", "name", "product_key"],
    )
    .unwrap();
    pub static ref LICMAN20_FEATURES_USER: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "licman20_feature_used_users",
            "Number of licenses used by user"
        ),
        &["app", "name", "product_key", "user"],
    )
    .unwrap();
    pub static ref LICMAN20_FEATURE_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "licman20_feature_expiration_seconds",
            "Time until license features will expire"
        ),
        &["app", "index", "licenses", "name", "product_key"]
    )
    .unwrap();
    pub static ref LICMAN20_FEATURE_AGGREGATED_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "licman20_feature_aggregate_expiration_seconds",
            "Aggregated licenses by expiration time"
        ),
        &["app", "features", "index", "licenses"]
    )
    .unwrap();
}

struct Licman20LicenseData {
    pub product_key: String,
    pub feature: String,
    pub total: i64,
    pub used: i64,
}

struct Licman20LicenseExpiration {
    pub product_key: String,
    pub feature: String,
    pub license_count: i64,
    pub expiration: f64,
}

pub fn fetch(lic: &config::Licman20, licman20_appl: &str) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_LICMAN20_PRODUCT_KEY: Regex =
            Regex::new(r"^Product key\s+:\s+(\d+)$").unwrap();
        static ref RE_LICMAN20_TOTAL_LICENSES: Regex =
            Regex::new(r"^Number of Licenses\s+:\s+(\d+)$").unwrap();
        static ref RE_LICMAN20_USED_LICENSES: Regex = Regex::new(r"^In use\s+:\s+(\d+)$").unwrap();
        static ref RE_LICMAN20_END_DATE: Regex = Regex::new(r"^End date\s+:\s+([\w\-]+)$").unwrap();
        static ref RE_LICMAN20_FEATURE: Regex = Regex::new(r"^Comment\s+:\s+(\w+)$").unwrap();
    }

    let mut licenses: Vec<Licman20LicenseData> = Vec::new();
    let mut expiring = Vec::<Licman20LicenseExpiration>::new();
    let mut aggregated_expiration: HashMap<String, Vec<Licman20LicenseExpiration>> = HashMap::new();
    let mut expiration_dates = Vec::<f64>::new();
    let mut product_key_map: HashMap<String, String> = HashMap::new();

    env::set_var("LANG", "C");
    debug!("licman20.rs:fetch: Running {}", licman20_appl);

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
    let mut product_key: &str = "";
    let mut total: i64 = 0;
    let mut used: i64 = 0;
    let mut expiration: f64 = 0.0;

    for line in stderr.lines() {
        if line.is_empty() {
            continue;
        }

        if let Some(capt) = RE_LICMAN20_PRODUCT_KEY.captures(line) {
            if capt.len() != 2 {
                error!(
                    "Regular expression returns {} capture groups instead of 2",
                    capt.len()
                );
                continue;
            }
            debug!(
                "licman20.rs:fetch: RE_LICMAN20_PRODUCT_KEY match on {}",
                line
            );

            // Flush collected data
            if !product_key.is_empty() {
                licenses.push(Licman20LicenseData {
                    product_key: product_key.to_string(),
                    feature: feature.to_string(),
                    total,
                    used,
                });

                expiration_dates.push(expiration);
                expiring.push(Licman20LicenseExpiration {
                    product_key: product_key.to_string(),
                    feature: feature.to_string(),
                    expiration,
                    license_count: total,
                });

                let expiration_str = expiration.to_string();
                let aggregated = aggregated_expiration
                    .entry(expiration_str)
                    .or_insert_with(Vec::<Licman20LicenseExpiration>::new);

                aggregated.push(Licman20LicenseExpiration {
                    product_key: product_key.to_string(),
                    feature: feature.to_string(),
                    expiration,
                    license_count: total,
                });

                product_key_map.insert(product_key.to_string(), feature.to_string());
            }

            product_key = capt.get(1).map_or("", |m| m.as_str());
        } else if let Some(capt) = RE_LICMAN20_FEATURE.captures(line) {
            if capt.len() != 2 {
                error!(
                    "Regular expression returns {} capture groups instead of 2",
                    capt.len()
                );
                continue;
            }
            debug!("licman20.rs:fetch: RE_LICMAN20_FEATURE match on {}", line);
            feature = capt.get(1).map_or("", |m| m.as_str());
        } else if let Some(capt) = RE_LICMAN20_TOTAL_LICENSES.captures(line) {
            if capt.len() != 2 {
                error!(
                    "Regular expression returns {} capture groups instead of 2",
                    capt.len()
                );
                continue;
            }
            debug!(
                "licman20.rs:fetch: RE_LICMAN20_TOTAL_LICENSES match on {}",
                line
            );
            let _total = capt.get(1).map_or("", |m| m.as_str());
            total = match _total.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as integer: {}", _total, e);
                    continue;
                }
            };
        } else if let Some(capt) = RE_LICMAN20_USED_LICENSES.captures(line) {
            if capt.len() != 2 {
                error!(
                    "Regular expression returns {} capture groups instead of 2",
                    capt.len()
                );
                continue;
            }
            debug!(
                "licman20.rs:fetch: RE_LICMAN20_USED_LICENSES match on {}",
                line
            );
            let _used = capt.get(1).map_or("", |m| m.as_str());
            used = match _used.parse() {
                Ok(v) => v,
                Err(e) => {
                    error!("Can't parse {} as integer: {}", _used, e);
                    continue;
                }
            };
        } else if let Some(capt) = RE_LICMAN20_END_DATE.captures(line) {
            if capt.len() != 2 {
                error!(
                    "Regular expression returns {} capture groups instead of 2",
                    capt.len()
                );
                continue;
            }
            debug!("licman20.rs:fetch: RE_LICMAN20_END_DATE match on {}", line);
            let end_date = capt.get(1).map_or("", |m| m.as_str());
            expiration = match NaiveDateTime::parse_from_str(
                &format!("{} 00:00:00", end_date),
                "%d-%b-%Y %H:%M:%S",
            ) {
                Ok(v) => v.timestamp() as f64,
                Err(e) => {
                    error!("Can't parse {} as date and time: {}", end_date, e);
                    continue;
                }
            };
        } else {
            debug!("licman20.rs:fetch: No regexp matches '{}'", line);
        }
    }

    // Push last collected entry
    if !product_key.is_empty() {
        licenses.push(Licman20LicenseData {
            product_key: product_key.to_string(),
            feature: feature.to_string(),
            total,
            used,
        });

        expiration_dates.push(expiration);
        expiring.push(Licman20LicenseExpiration {
            product_key: product_key.to_string(),
            feature: feature.to_string(),
            expiration,
            license_count: total,
        });

        let expiration_str = expiration.to_string();
        let aggregated = aggregated_expiration
            .entry(expiration_str)
            .or_insert_with(Vec::<Licman20LicenseExpiration>::new);

        aggregated.push(Licman20LicenseExpiration {
            product_key: product_key.to_string(),
            feature: feature.to_string(),
            expiration,
            license_count: total,
        });

        product_key_map.insert(product_key.to_string(), feature.to_string());
    }

    for l in licenses {
        if license::is_excluded(&lic.excluded_features, l.feature.to_string()) {
            debug!("licman20.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", l.feature, lic.name);
            continue;
        }
        debug!(
            "Setting licman20_feature_issued {} {} {} -> {}",
            lic.name, l.feature, l.product_key, l.total
        );
        LICMAN20_FEATURES_TOTAL
            .with_label_values(&[&lic.name, &l.feature, &l.product_key])
            .set(l.total);

        debug!(
            "Setting licman20_feature_used {} {} {} -> {}",
            lic.name, l.feature, l.product_key, l.used
        );
        LICMAN20_FEATURES_TOTAL
            .with_label_values(&[&lic.name, &l.feature, &l.product_key])
            .set(l.used);
    }

    let mut index: i64 = 1;
    for entry in expiring {
        if license::is_excluded(&lic.excluded_features, entry.feature.to_string()) {
            debug!("licman20.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", entry.feature, lic.name);
            continue;
        }

        debug!(
            "licman20.rs:fetch: Setting licman20_feature_used_users {} {} {} {} {} -> {}",
            lic.name,
            index,
            entry.license_count.to_string(),
            entry.feature,
            entry.product_key,
            entry.expiration
        );
        LICMAN20_FEATURE_EXPIRATION
            .with_label_values(&[
                &lic.name,
                &index.to_string(),
                &entry.license_count.to_string(),
                &entry.product_key,
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
            debug!("licman20.rs:fetch_expiration: Setting licman20_feature_aggregate_expiration_seconds {} {} {} {} -> {}", lic.name, feature_count, index, license_count, exp);
            LICMAN20_FEATURE_AGGREGATED_EXPIRATION
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

    if let Some(export_users) = lic.export_user {
        if export_users {
            match fetch_checkouts(lic, licman20_appl, &product_key_map) {
                Ok(_) => {}
                Err(e) => {
                    error!("Unable to get license checkouts: {}", e);
                }
            }
        }
    }

    Ok(())
}

fn fetch_checkouts(
    lic: &config::Licman20,
    licman20_appl: &str,
    pmap: &HashMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_LICMAN20_CHECKOUT: Regex =
            Regex::new(r"^\d{2}/\d{2}/\d{2}\s\d{2}:\d{2}:\d{2}\s+([\w_\-.]+)\s+(\d+)\s*.*$")
                .unwrap();
    }

    let mut fu: HashMap<String, HashMap<String, i64>> = HashMap::new();

    env::set_var("LANG", "C");
    debug!("licman20.rs:fetch_checkouts: Running {}", licman20_appl);

    let mut cmd = Command::new(licman20_appl)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    cmd.stdin
        .as_mut()
        .ok_or("Unable to connect to stdin for command")?
        .write_all(b"2\nX\n")?;

    let stdout_and_err = cmd.wait_with_output()?;

    let rc = match stdout_and_err.status.code() {
        Some(v) => v,
        None => {
            bail!("Can't get return code of {} command", licman20_appl);
        }
    };
    debug!(
        "licman20.rs:fetch_checkouts: external command finished with exit code {}",
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

    for line in stderr.lines() {
        if line.is_empty() {
            continue;
        }

        if let Some(capt) = RE_LICMAN20_CHECKOUT.captures(line) {
            if capt.len() != 3 {
                error!(
                    "Regular expression returns {} capture groups instead of 3",
                    capt.len()
                );
                continue;
            }
            debug!(
                "licman20.rs:fetch_checkouts: RE_LICMAN20_CHECKOUT match on {}",
                line
            );

            let user = capt.get(1).map_or("", |m| m.as_str());
            let product_key = capt.get(2).map_or("", |m| m.as_str());

            let usr = fu
                .entry(product_key.to_string())
                .or_insert_with(HashMap::<String, i64>::new);
            *usr.entry(user.to_string()).or_insert(0) += 1;
        } else {
            debug!("licman20.rs:fetch_checkouts: No regexp matches '{}'", line);
        }
    }

    for (feat, uv) in fu.iter() {
        let fname = match pmap.get(feat) {
            Some(v) => v,
            None => feat,
        };

        for (user, count) in uv.iter() {
            if license::is_excluded(&lic.excluded_features, feat.to_string()) {
                debug!("licman20.rs:fetch_checkouts: Skipping product_key {} because it is in excluded_features list of {}", feat, lic.name);
                continue;
            }
            debug!(
                "licman20.rs:fetch_checkouts: Setting licman20_feature_used_users {} {} {} {} -> {}",
                lic.name, fname, feat, user, *count
            );
            LICMAN20_FEATURES_USER
                .with_label_values(&[&lic.name, fname, feat, user])
                .set(*count);
        }
    }

    Ok(())
}

pub fn register() {
    exporter::REGISTRY
        .register(Box::new(LICMAN20_FEATURES_TOTAL.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LICMAN20_FEATURES_USED.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LICMAN20_FEATURES_USER.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LICMAN20_FEATURE_EXPIRATION.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LICMAN20_FEATURE_AGGREGATED_EXPIRATION.clone()))
        .unwrap();
}
