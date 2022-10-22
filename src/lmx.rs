use crate::config;
use crate::exporter;
use crate::license;

use chrono::NaiveDateTime;
use lazy_static::lazy_static;
use log::{debug, error, warn};
use prometheus::{GaugeVec, IntGaugeVec, Opts};
use quick_xml::events::Event;
use quick_xml::Reader;
use simple_error::bail;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::process::Command;
use std::str;

lazy_static! {
    pub static ref LMX_FEATURES_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("lmx_feature_issued", "Total number of issued licenses"),
        &["app", "name"],
    )
    .unwrap();
    pub static ref LMX_FEATURES_USED: IntGaugeVec = IntGaugeVec::new(
        Opts::new("lmx_feature_used", "Number of used licenses"),
        &["app", "name"],
    )
    .unwrap();
    pub static ref LMX_FEATURES_DENIED: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "lmx_feature_denied",
            "Total number of denied license checkouts"
        ),
        &["app", "name"],
    )
    .unwrap();
    pub static ref LMX_SERVER_STATUS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("lmx_server_status", "Status of license server(s)"),
        &["app", "fqdn", "master", "port", "version"],
    )
    .unwrap();
    pub static ref LMX_FEATURES_USER: IntGaugeVec = IntGaugeVec::new(
        Opts::new("lmx_feature_used_users", "Number of licenses used by user"),
        &["app", "name", "user", "version"],
    )
    .unwrap();
    pub static ref LMX_FEATURE_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "lmx_feature_expiration_seconds",
            "Time until license features will expire"
        ),
        &["app", "index", "licenses", "name", "vendor", "version"]
    )
    .unwrap();
    pub static ref LMX_FEATURE_AGGREGATED_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "lmx_feature_aggregate_expiration_seconds",
            "Aggregated licenses by expiration time"
        ),
        &["app", "features", "index", "licenses"]
    )
    .unwrap();
}

pub struct LmxLicenseExpiration {
    pub feature: String,
    pub version: String,
    pub vendor: String,
    pub license_count: i64,
    pub expiration: f64,
}

#[derive(Debug)]
struct LmxLicenseData {
    pub server_version: String,
    pub server_status: String,
    pub features: Vec<LmxLicenseFeatures>,
}

impl LmxLicenseData {
    pub fn new() -> Self {
        LmxLicenseData {
            server_version: String::new(),
            server_status: String::new(),
            features: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct LmxLicenseFeatures {
    pub feature: String,
    pub version: String,
    pub vendor: String,
    pub expiration_str: String,
    pub used: i64,
    pub total: i64,
    pub denied: i64,
    pub checkouts: Vec<LmxLicenseCheckouts>,
}

impl LmxLicenseFeatures {
    pub fn new() -> Self {
        LmxLicenseFeatures {
            feature: String::new(),
            version: String::new(),
            vendor: String::new(),
            expiration_str: String::new(),
            used: 0,
            total: 0,
            denied: 0,
            checkouts: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct LmxLicenseCheckouts {
    pub user: String,
    pub used: i64,
}

pub fn fetch(lic: &config::Lmx, lmxendutil: &str) -> Result<(), Box<dyn Error>> {
    // dict -> "feature" -> "user" -> "version" -> count
    let mut fuv: HashMap<String, HashMap<String, HashMap<String, i64>>> = HashMap::new();
    let mut server_port: HashMap<String, String> = HashMap::new();
    let mut server_master: HashMap<String, bool> = HashMap::new();

    for (i, lserver) in lic.license.split(':').enumerate() {
        let mut port = "6200".to_string();
        let srv: String;

        if lserver.contains('@') {
            let srvport: Vec<&str> = lserver.split('@').collect();
            // NOTE: Configuration validation checks for valid server lines
            port = srvport[0].to_string();
            srv = srvport[1].to_string();
        } else {
            srv = lserver.to_string();
        }
        server_port.insert(srv.clone(), port);
        match i {
            0 => {
                server_master.insert(srv.clone(), true);
            }
            _ => {
                server_master.insert(srv.clone(), false);
            }
        };
    }

    /*
      Note: Due to the HA method of LM-X we will not process data returned from all other servers if we already
            receive license data from a previous server.
            "
            High Availability Licensing (HAL) servers, which enable redundant servers, so if one server goes down,
            two others will still work. HAL consists of 3 specified servers, at least 2 of which must be up and
            running at all times.

            Each HAL_SERVER line indicates a license server that has HAL enabled by its license(s). Each HAL server
            has a specific role, and should be specified in terms of how many resources each server has:

                HAL_SERVER1 is your master server, which allows both CHECKOUT and BORROW.
                HAL_SERVER1 should be your most powerful server.

                HAL_SERVER2 is your first slave server, which allows CHECKOUT but denies BORROW in the event that
                your master server goes down.
                HAL_SERVER2 should be your second most powerful server.

                HAL_SERVER3 is part of your configuration to ensure that everything works as expected, and does not
                allow any CHECKOUT or BORROW requests.
                HAL_SERVER3 should be your least powerful server.
            "
            (see https://docs.x-formation.com/display/LMX/License+server+configuration+file)
    */
    let mut server_is_ok: bool;
    let mut features_exported = false;

    for (server, port) in server_port {
        env::set_var("LANG", "C");
        debug!(
            "lmx.rs:fetch: Running {} -licstatxml -host {} -port {}",
            lmxendutil, server, port
        );
        let cmd = Command::new(lmxendutil)
            .arg("-licstatxml")
            .arg("-host")
            .arg(&server)
            .arg("-port")
            .arg(&port)
            .output()?;

        let rc = match cmd.status.code() {
            Some(v) => v,
            None => {
                bail!("Can't get return code of {} command", lmxendutil);
            }
        };
        debug!(
            "lmx.rs:fetch: external command finished with exit code {}",
            rc
        );

        if !cmd.status.success() {
            bail!(
                "{} command exited with non-normal exit code {} for {}",
                lmxendutil,
                rc,
                lic.name
            );
        }

        let stdout = String::from_utf8(cmd.stdout)?;
        let parsed = parse_xml(stdout)?;

        let _master = server_master.get(&server).unwrap_or(&false);
        let master = format!("{}", _master);

        if parsed.server_status == "SUCCESS" {
            debug!(
                "lmx.rs:fetch: Setting lmx_server_status {} {} {} {} {} -> 1",
                lic.name, server, master, port, parsed.server_version
            );
            LMX_SERVER_STATUS
                .with_label_values(&[&lic.name, &server, &master, &port, &parsed.server_version])
                .set(1);
            server_is_ok = true;
        } else {
            debug!(
                "lmx.rs:fetch: Setting lmx_server_status {} {} {} {} {} -> 0",
                lic.name, server, master, port, parsed.server_version
            );
            LMX_SERVER_STATUS
                .with_label_values(&[&lic.name, &server, &master, &port, &parsed.server_version])
                .set(0);
            server_is_ok = false;
        }

        // Only report feature usage for a healthy server
        if !server_is_ok {
            continue;
        }

        // Only export feature usage once
        if features_exported {
            continue;
        }

        let mut expiring = Vec::<LmxLicenseExpiration>::new();
        let mut aggregated_expiration: HashMap<String, Vec<LmxLicenseExpiration>> = HashMap::new();
        let mut expiration_dates = Vec::<f64>::new();

        for feature in parsed.features {
            if license::is_excluded(&lic.excluded_features, feature.feature.clone()) {
                debug!("lmx.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", feature.feature, lic.name);
                continue;
            }

            debug!(
                "lmx.rs:fetch: Setting lmx_feature_issued {} {} -> {}",
                lic.name, feature.feature, feature.total
            );
            LMX_FEATURES_TOTAL
                .with_label_values(&[&lic.name, &feature.feature])
                .set(feature.total);
            debug!(
                "lmx.rs:fetch: Setting lmx_feature_used {} {} -> {}",
                lic.name, feature.feature, feature.total
            );
            LMX_FEATURES_USED
                .with_label_values(&[&lic.name, &feature.feature])
                .set(feature.used);
            debug!(
                "lmx.rs:fetch: Setting lmx_feature_denied {} {} -> {}",
                lic.name, feature.feature, feature.denied
            );
            LMX_FEATURES_DENIED
                .with_label_values(&[&lic.name, &feature.feature])
                .set(feature.denied);

            for co in feature.checkouts {
                let feat = fuv
                    .entry(feature.feature.to_string())
                    .or_insert_with(HashMap::<String, HashMap<String, i64>>::new);
                let usr = feat
                    .entry(co.user.to_string())
                    .or_insert_with(HashMap::<String, i64>::new);
                *usr.entry(feature.version.to_string()).or_insert(0) += co.used;
            }

            let expiration: f64 = match NaiveDateTime::parse_from_str(
                &format!("{} 00:00:00", feature.expiration_str),
                "%Y-%m-%d %H:%M:%S",
            ) {
                Ok(v) => v.timestamp() as f64,
                Err(e) => {
                    error!(
                        "lmx.rs:fetch: Can't parse {} as date and time: {}",
                        feature.expiration_str, e
                    );
                    continue;
                }
            };
            expiration_dates.push(expiration);
            expiring.push(LmxLicenseExpiration {
                feature: feature.feature.to_string(),
                version: feature.version.to_string(),
                vendor: feature.vendor.to_string(),
                license_count: feature.total,
                expiration,
            });

            let expiration_str = expiration.to_string();
            let aggregated = aggregated_expiration
                .entry(expiration_str)
                .or_insert_with(Vec::<LmxLicenseExpiration>::new);
            aggregated.push(LmxLicenseExpiration {
                feature: feature.feature.to_string(),
                version: feature.version.to_string(),
                vendor: feature.vendor.to_string(),
                license_count: feature.total,
                expiration,
            });
        }

        if let Some(export_user) = lic.export_user {
            if export_user {
                for (feat, uv) in fuv.iter() {
                    for (user, v) in uv.iter() {
                        for (version, count) in v.iter() {
                            if license::is_excluded(&lic.excluded_features, feat.to_string()) {
                                debug!("lmx.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", feat, lic.name);
                                continue;
                            }
                            debug!(
                                "lmx.rs:fetch: Setting lmx_feature_used_users {} {} {} {} -> {}",
                                lic.name, feat, user, version, *count
                            );
                            LMX_FEATURES_USER
                                .with_label_values(&[&lic.name, feat, user, version])
                                .set(*count);
                        }
                    }
                }
            }
        }

        let mut index: i64 = 1;
        for entry in expiring {
            if license::is_excluded(&lic.excluded_features, entry.feature.to_string()) {
                debug!("lmx.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", entry.feature, lic.name);
                continue;
            }

            debug!(
                "lmx.rs:fetch: Setting lmx_feature_used_users {} {} {} {} {} {} -> {}",
                lic.name,
                index,
                entry.license_count.to_string(),
                entry.feature,
                entry.vendor,
                entry.version,
                entry.expiration
            );
            LMX_FEATURE_EXPIRATION
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
                debug!("lmx.rs:fetch_expiration: Setting lmx_feature_aggregate_expiration_seconds {} {} {} {} -> {}", lic.name, feature_count, index, license_count, exp);
                LMX_FEATURE_AGGREGATED_EXPIRATION
                    .with_label_values(&[
                        &lic.name,
                        &feature_count.to_string(),
                        &index.to_string(),
                        &license_count.to_string(),
                    ])
                    .set(exp);
                index += 1;
            } else {
                warn!(
                    "lmx.rs:fetch_expiration: Key {} not found in HashMap aggregated",
                    exp_str
                );
            }
        }

        features_exported = true;
    }

    Ok(())
}

fn parse_xml(raw: String) -> Result<LmxLicenseData, Box<dyn Error>> {
    let mut result = LmxLicenseData::new();
    let mut reader = Reader::from_str(&raw);
    let mut buffer = Vec::new();
    let mut feature = LmxLicenseFeatures::new();
    let mut _fname = String::new();

    reader.trim_text(true);

    // XXX: This looks messy but quick_xml gets this job done fast and flexible
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(v)) | Ok(Event::Empty(v)) => {
                let _tag_name_v = v.name();
                let tag_name = _tag_name_v.as_ref();
                match tag_name {
                    // e.g. <LICENSE_PATH TYPE="NETWORK" HOST="6200@server1" SERVER_VERSION="5.5" UPTIME="8 hour(s) 38 min(s) 33 sec(s)" STATUS="SUCCESS">
                    b"LICENSE_PATH" => {
                        for attribute in v.attributes() {
                            match attribute {
                                Ok(attr) => {
                                    let _key_ln = attr.key.local_name();
                                    let key = str::from_utf8(_key_ln.as_ref())?;
                                    let value = str::from_utf8(&attr.value)?;
                                    match key {
                                        "SERVER_VERSION" => {
                                            result.server_version = value.to_string();
                                        }
                                        "STATUS" => {
                                            result.server_status = value.to_string();
                                        }
                                        _ => {}
                                    };
                                }
                                Err(e) => {
                                    return Err(Box::new(e));
                                }
                            };
                        }
                    }
                    // e.g. <FEATURE NAME="FeatUre" VERSION="22.0" VENDOR="ALTAIR" START="2020-04-25" END="2025-12-31" USED_LICENSES="655000" TOTAL_LICENSES="1150000" DENIED_LICENSES="0" SHARE="CUSTOM ,VIRTUAL">
                    b"FEATURE" => {
                        let mut feature_name = String::new();
                        let mut feature_version = String::new();
                        let mut feature_expiration = String::new();
                        let mut feature_vendor = String::new();
                        let mut feature_used: i64 = 0;
                        let mut feature_total: i64 = 0;
                        let mut feature_denied: i64 = 0;

                        for attribute in v.attributes() {
                            match attribute {
                                Ok(attr) => {
                                    let _key_ln = attr.key.local_name();
                                    let key = str::from_utf8(_key_ln.as_ref())?;
                                    let value = str::from_utf8(&attr.value)?;

                                    match key {
                                        "NAME" => {
                                            if !_fname.is_empty() {
                                                result.features.push(feature.clone());
                                            }
                                            feature_name = value.to_string();
                                            _fname = feature_name.clone();
                                        }
                                        "VERSION" => {
                                            feature_version = value.to_string();
                                        }
                                        "VENDOR" => {
                                            feature_vendor = value.to_string();
                                        }
                                        "END" => {
                                            feature_expiration = value.to_string();
                                        }
                                        "USED_LICENSES" => {
                                            feature_used = value.parse()?;
                                        }
                                        "TOTAL_LICENSES" => {
                                            feature_total = value.parse()?;
                                        }
                                        "DENIED_LICENSES" => {
                                            feature_denied = value.parse()?;
                                        }
                                        _ => {}
                                    };
                                }
                                Err(e) => {
                                    return Err(Box::new(e));
                                }
                            };
                        }
                        feature = LmxLicenseFeatures {
                            feature: feature_name.clone(),
                            version: feature_version.clone(),
                            vendor: feature_vendor.clone(),
                            expiration_str: feature_expiration.clone(),
                            used: feature_used,
                            total: feature_total,
                            denied: feature_denied,
                            checkouts: Vec::new(),
                        };
                    }
                    // e.g.  <USER NAME="user1" HOST="client1" IP="253.255.250.288" USED_LICENSES="21000" LOGIN_TIME="2022-02-01 15:12" CHECKOUT_TIME="2022-02-01 15:12" SHARE_CUSTOM="user1:client1"/>
                    b"USER" => {
                        let mut user = String::new();
                        let mut used: i64 = 0;

                        for attribute in v.attributes() {
                            match attribute {
                                Ok(attr) => {
                                    let _key_ln = attr.key.local_name();
                                    let key = str::from_utf8(_key_ln.as_ref())?;
                                    let value = str::from_utf8(&attr.value)?;

                                    match key {
                                        "NAME" => {
                                            user = value.to_string();
                                        }
                                        "USED_LICENSES" => {
                                            used = value.parse()?;
                                        }
                                        _ => {}
                                    };
                                }
                                Err(e) => {
                                    return Err(Box::new(e));
                                }
                            };
                        }

                        feature.checkouts.push(LmxLicenseCheckouts { user, used });
                    }
                    _ => {}
                };
            }
            Ok(Event::Eof) => {
                break;
            }
            Err(e) => {
                return Err(Box::new(e));
            }
            _ => {}
        }
    }
    Ok(result)
}

pub fn register() {
    exporter::REGISTRY
        .register(Box::new(LMX_SERVER_STATUS.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LMX_FEATURES_USED.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LMX_FEATURES_DENIED.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LMX_FEATURES_TOTAL.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LMX_FEATURES_USER.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LMX_FEATURE_EXPIRATION.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(LMX_FEATURE_AGGREGATED_EXPIRATION.clone()))
        .unwrap();
}
