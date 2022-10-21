use crate::config;
use crate::constants;
use crate::exporter;
use crate::http;
use crate::license;

use chrono::NaiveDateTime;
use lazy_static::lazy_static;
use log::{debug, error, warn};
use prometheus::{GaugeVec, IntGaugeVec, Opts};
use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use simple_error::bail;
use std::collections::HashMap;
use std::error::Error;
use std::str;

lazy_static! {
    pub static ref OLICENSE_SERVER_STATUS: IntGaugeVec = IntGaugeVec::new(
        Opts::new("olicense_server_status", "Status of license server"),
        &["app", "fqdn", "port", "version"],
    )
    .unwrap();
    pub static ref OLICENSE_FEATURES_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("olicense_feature_issued", "Total number of issued licenses"),
        &["app", "vendor", "name", "module"],
    )
    .unwrap();
    pub static ref OLICENSE_FEATURES_USED: IntGaugeVec = IntGaugeVec::new(
        Opts::new("olicense_feature_used", "Number of used licenses"),
        &["app", "vendor", "name", "module"],
    )
    .unwrap();
    pub static ref OLICENSE_FEATURES_USER: IntGaugeVec = IntGaugeVec::new(
        Opts::new(
            "olicense_feature_used_users",
            "Number of licenses used by user"
        ),
        &["app", "name", "user", "version"],
    )
    .unwrap();
    pub static ref OLICENSE_FEATURE_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "olicense_feature_expiration_seconds",
            "Time until license features will expire"
        ),
        &["app", "index", "licenses", "name", "module", "vendor", "version"]
    )
    .unwrap();
    pub static ref OLICENSE_FEATURE_AGGREGATED_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "olicense_feature_aggregate_expiration_seconds",
            "Aggregated licenses by expiration time"
        ),
        &["app", "features", "index", "licenses"]
    )
    .unwrap();
}

#[derive(Clone, Debug)]
struct OLicenseData {
    pub server_version: String,
    pub features: Vec<OLicenseFeature>,
}

impl OLicenseData {
    pub fn new() -> Self {
        OLicenseData {
            server_version: String::new(),
            features: Vec::new(),
        }
    }
}

// Keep track of the XML tags we are operating in
const OLIC_TAG_NAME: u8 = 0x01;
const OLIC_TAG_MODULE: u8 = 0x02;
const OLIC_TAG_MODULE_NAME: u8 = 0x03;
const OLIC_TAG_VENDOR: u8 = 0x04;
const OLIC_TAG_TOTAL: u8 = 0x05;
const OLIC_TAG_USED: u8 = 0x06;
const OLIC_TAG_EXPIRATION_DATE: u8 = 0x07;
const OLIC_TAG_SERVER_VERSION: u8 = 0x08;
const OLIC_TAG_CHECKOUTS: u8 = 0x09;
const OLIC_TAG_VERSION_RANGE: u8 = 0x0a;

#[derive(Clone, Debug)]
struct OLicenseFeature {
    pub name: String,
    pub module: String,
    pub vendor: String,
    pub total: i64,
    pub used: i64,
    pub expiration_date: String,
    pub expiration: f64,
    pub checkouts: Vec<OLicenseCheckout>,
    pub version_range: String,
}

#[derive(Clone, Debug)]
struct OLicenseCheckout {
    pub user: String,
    pub count: i64,
}

impl OLicenseFeature {
    pub fn new() -> Self {
        OLicenseFeature {
            name: String::new(),
            module: String::new(),
            vendor: String::new(),
            total: 0,
            used: 0,
            expiration_date: String::new(),
            expiration: 0.0,
            checkouts: Vec::<OLicenseCheckout>::new(),
            version_range: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct OLicenseExpiration {
    pub feature: String,
    pub version: String,
    pub vendor: String,
    pub module: String,
    pub license_count: i64,
    pub expiration: f64,
}

pub fn fetch(lic: &config::Olicense) -> Result<(), Box<dyn Error>> {
    // dict -> "feature" -> "user" -> "version" -> count
    let mut fuv: HashMap<String, HashMap<String, HashMap<String, i64>>> = HashMap::new();
    let mut server_port: HashMap<String, String> = HashMap::new();
    let mut server_master: HashMap<String, bool> = HashMap::new();
    let mut http_client = http::build_client(false, "", constants::DEFAULT_TIMEOUT)?;

    for (i, lserver) in lic.license.split(':').enumerate() {
        let mut port = "8080".to_string();
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

    let mut server_is_ok: bool;
    let mut features_exported = false;

    for (server, port) in server_port {
        let url = format!("http://{}:{}/LicenseStatusXML", server, port);

        let reply = match http::get(&mut http_client, &url, "", "") {
            Ok(v) => v,
            Err(e) => {
                error!(
                    "olicense.rs:fetch: Can't fetch license information from OLicense server {}:{}: {}",
                    server, port, e
                );
                debug!(
                    "Setting olicense_server_status {} {} {} {} -> 0",
                    lic.name, server, port, "",
                );
                OLICENSE_SERVER_STATUS
                    .with_label_values(&[&lic.name, &server, &port, ""])
                    .set(0);
                continue;
            }
        };

        let parsed = match parse_xml(reply) {
            Ok(v) => v,
            Err(e) => {
                error!(
                    "olicense.rs:fetch: Can't parse license information from OLicense server {}:{}: {}",
                    server, port, e
                );
                debug!(
                    "Setting olicense_server_status {} {} {} {} -> 0",
                    lic.name, server, port, ""
                );
                OLICENSE_SERVER_STATUS
                    .with_label_values(&[&lic.name, &server, &port, ""])
                    .set(0);
                continue;
            }
        };
        debug!("{:?}", parsed);

        server_is_ok = true;

        // Only report feature usage for a healthy server
        if !server_is_ok {
            continue;
        }

        debug!(
            "Setting olicense_server_status {} {} {} {} -> 1",
            lic.name, server, port, parsed.server_version,
        );
        OLICENSE_SERVER_STATUS
            .with_label_values(&[&lic.name, &server, &port, &parsed.server_version])
            .set(1);

        // Only export feature usage once
        if features_exported {
            continue;
        }

        let mut expiring = Vec::<OLicenseExpiration>::new();
        let mut aggregated_expiration: HashMap<String, Vec<OLicenseExpiration>> = HashMap::new();
        let mut expiration_dates = Vec::<f64>::new();

        for f in parsed.features {
            if license::is_excluded(&lic.excluded_features, f.name.clone()) {
                debug!("olicense.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", f.name, lic.name);
                continue;
            }

            debug!(
                "Setting olicense_feature_issued {} {} {} {} -> {}",
                lic.name, f.vendor, f.name, f.module, f.total
            );
            OLICENSE_FEATURES_TOTAL
                .with_label_values(&[&lic.name, &f.vendor, &f.name, &f.module])
                .set(f.total);

            debug!(
                "Setting olicense_feature_used {} {} {} {} -> {}",
                lic.name, f.vendor, f.name, f.module, f.used
            );
            OLICENSE_FEATURES_USED
                .with_label_values(&[&lic.name, &f.vendor, &f.name, &f.module])
                .set(f.used);

            for co in f.checkouts {
                let feat = fuv
                    .entry(f.name.to_string())
                    .or_insert_with(HashMap::<String, HashMap<String, i64>>::new);
                let usr = feat
                    .entry(co.user.to_string())
                    .or_insert_with(HashMap::<String, i64>::new);
                *usr.entry(f.version_range.to_string()).or_insert(0) += co.count;
            }

            expiration_dates.push(f.expiration);
            expiring.push(OLicenseExpiration {
                feature: f.name.to_string(),
                version: f.version_range.to_string(),
                vendor: f.vendor.to_string(),
                module: f.module.to_string(),
                license_count: f.total,
                expiration: f.expiration,
            });

            let expiration_str = f.expiration.to_string();
            let aggregated = aggregated_expiration
                .entry(expiration_str)
                .or_insert_with(Vec::<OLicenseExpiration>::new);
            aggregated.push(OLicenseExpiration {
                feature: f.name.to_string(),
                version: f.version_range.to_string(),
                vendor: f.vendor.to_string(),
                module: f.module.to_string(),
                license_count: f.total,
                expiration: f.expiration,
            });
        }

        if let Some(export_user) = lic.export_user {
            if export_user {
                for (feat, uv) in fuv.iter() {
                    for (user, v) in uv.iter() {
                        for (version, count) in v.iter() {
                            if license::is_excluded(&lic.excluded_features, feat.to_string()) {
                                debug!("olicense.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", feat, lic.name);
                                continue;
                            }
                            debug!(
                                "olicense.rs:fetch: Setting olicense_feature_used_users {} {} {} {} -> {}",
                                lic.name, feat, user, version, *count
                            );
                            OLICENSE_FEATURES_USER
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
                debug!("olicense.rs:fetch: Skipping feature {} because it is in excluded_features list of {}", entry.feature, lic.name);
                continue;
            }

            debug!(
                "olicense.rs:fetch: Setting olicense_feature_used_users {} {} {} {} {} {} {} -> {}",
                lic.name,
                index,
                entry.license_count.to_string(),
                entry.feature,
                entry.module,
                entry.vendor,
                entry.version,
                entry.expiration
            );
            OLICENSE_FEATURE_EXPIRATION
                .with_label_values(&[
                    &lic.name,
                    &index.to_string(),
                    &entry.license_count.to_string(),
                    &entry.feature,
                    &entry.module,
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
                debug!("olicense.rs:fetch_expiration: Setting olicense_feature_aggregate_expiration_seconds {} {} {} {} -> {}", lic.name, feature_count, index, license_count, exp);
                OLICENSE_FEATURE_AGGREGATED_EXPIRATION
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

        features_exported = true;
    }

    Ok(())
}

fn parse_xml(raw: String) -> Result<OLicenseData, Box<dyn Error>> {
    let mut result = OLicenseData::new();
    let mut reader = Reader::from_str(&raw);
    let mut buffer = Vec::new();
    let mut feature = OLicenseFeature::new();
    let mut _fname = String::new();
    let mut xml_tag: u8 = 0;

    reader.trim_text(true);

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(v)) | Ok(Event::Empty(v)) => {
                let _tag_name = v.name();
                let tag_name = _tag_name.as_ref();
                match tag_name {
                    b"serverVersion" => {
                        xml_tag = OLIC_TAG_SERVER_VERSION;
                    }
                    b"license" => {
                        feature = OLicenseFeature::new();
                    }
                    b"name" => {
                        if xml_tag == OLIC_TAG_MODULE {
                            xml_tag = OLIC_TAG_MODULE_NAME;
                        } else {
                            xml_tag = OLIC_TAG_NAME;
                        }
                    }
                    b"module" => {
                        xml_tag = OLIC_TAG_MODULE;
                    }
                    b"licenser" => {
                        xml_tag = OLIC_TAG_VENDOR;
                    }
                    b"floatCount" => {
                        xml_tag = OLIC_TAG_TOTAL;
                    }
                    b"floatsLocked" => {
                        xml_tag = OLIC_TAG_USED;
                    }
                    b"expiration" => {
                        xml_tag = OLIC_TAG_EXPIRATION_DATE;
                    }
                    b"floatsLockedBy" => {
                        xml_tag = OLIC_TAG_CHECKOUTS;
                    }
                    b"versionRange" => {
                        xml_tag = OLIC_TAG_VERSION_RANGE;
                    }
                    _ => {}
                };
            }
            Ok(Event::End(v)) => {
                let _tag_name = v.name();
                let tag_name = _tag_name.as_ref();
                if let b"license" = tag_name {
                    result.features.push(feature.clone());
                };
                xml_tag = 0;
            }
            Ok(Event::Text(txt)) => {
                let value = txt.unescape()?;
                match xml_tag {
                    OLIC_TAG_NAME => {
                        feature.name = value.to_string().clone();
                    }
                    OLIC_TAG_MODULE_NAME => {
                        feature.module = value.to_string().clone();
                    }
                    OLIC_TAG_USED => {
                        feature.used = value.parse()?;
                    }
                    OLIC_TAG_TOTAL => {
                        feature.total = value.parse()?;
                    }
                    OLIC_TAG_VENDOR => {
                        feature.vendor = value.to_string().clone();
                    }
                    OLIC_TAG_EXPIRATION_DATE => {
                        feature.expiration_date = value.to_string().clone();
                        feature.expiration = match NaiveDateTime::parse_from_str(
                            &format!("{} 00:00:00", value.to_string().clone()),
                            "%Y-%m-%d %H:%M:%S",
                        ) {
                            Ok(v) => v.timestamp() as f64,
                            Err(e) => {
                                bail!(
                                    "Can't parse {} as date and time: {}",
                                    feature.expiration_date,
                                    e
                                );
                            }
                        };
                    }
                    OLIC_TAG_SERVER_VERSION => {
                        result.server_version = value.to_string().clone();
                    }
                    OLIC_TAG_CHECKOUTS => {
                        feature.checkouts = parse_checkouts(value.to_string().clone())?;
                    }
                    OLIC_TAG_VERSION_RANGE => {
                        feature.version_range = value.to_string().clone();
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

fn parse_checkouts(raw: String) -> Result<Vec<OLicenseCheckout>, Box<dyn Error>> {
    lazy_static! {
        static ref RE_CHECKOUT: Regex =
            Regex::new(r"^([a-zA-Z0-9_\-.+]*)@([a-zA-Z0-9._\-]+)\s+#(\d+)$").unwrap();
    }
    let mut result: Vec<OLicenseCheckout> = Vec::new();
    // Format of checkouts is a comma separated list of <user>@<host> #<count>
    // where sometimes <user> is not set at all (why ?!?)
    let co_list: Vec<&str> = raw.split(',').collect();

    for co in co_list {
        if let Some(capt) = RE_CHECKOUT.captures(co.trim()) {
            let user = capt.get(1).map_or("", |m| m.as_str());
            let count: i64 = capt.get(3).map_or("", |m| m.as_str()).parse()?;

            result.push(OLicenseCheckout {
                user: user.to_string(),
                count,
            });
        }
    }

    Ok(result)
}

pub fn register() {
    exporter::REGISTRY
        .register(Box::new(OLICENSE_SERVER_STATUS.clone()))
        .unwrap();

    exporter::REGISTRY
        .register(Box::new(OLICENSE_FEATURES_USED.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(OLICENSE_FEATURES_TOTAL.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(OLICENSE_FEATURES_USER.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(OLICENSE_FEATURE_EXPIRATION.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(OLICENSE_FEATURE_AGGREGATED_EXPIRATION.clone()))
        .unwrap();
}
