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
use simple_error::bail;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::process::Command;
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

#[derive(Clone, Debug)]
struct OLicenseFeature {
    pub name: String,
    pub module: String,
    pub vendor: String,
    pub total: i64,
    pub used: i64,
    pub expiration_date: String,
    pub raw_checkouts: String,
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
            raw_checkouts: String::new(),
        }
    }
}

pub fn fetch(lic: &config::Olicense) -> Result<(), Box<dyn Error>> {
    // dict -> "feature" -> "user" -> "version" -> count
    /*    let mut fuv: HashMap<String, HashMap<String, HashMap<String, i64>>> = HashMap::new();
        let mut server_port: HashMap<String, String> = HashMap::new();
        let mut server_master: HashMap<String, bool> = HashMap::new();
    */

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
                    "Can't fetch license information from OLicense server {}:{}: {}",
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
                    "Can't parse license information from OLicense server {}:{}: {}",
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

        println!("{:?}", parsed);

        for f in parsed.features {
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
        }

        debug!(
            "Setting olicense_server_status {} {} {} {} -> 1",
            lic.name, server, port, parsed.server_version,
        );
        OLICENSE_SERVER_STATUS
            .with_label_values(&[&lic.name, &server, &port, &parsed.server_version])
            .set(1);
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
        match reader.read_event(&mut buffer) {
            Ok(Event::Start(v)) | Ok(Event::Empty(v)) => {
                let tag_name = v.name();
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
                    _ => {}
                };
            }
            Ok(Event::End(v)) => {
                let tag_name = v.name();
                match tag_name {
                    b"license" => {
                        // add license feature to list
                        result.features.push(feature.clone());
                    }
                    _ => {}
                }
                xml_tag = 0;
            }
            Ok(Event::Text(txt)) => {
                let value = txt.unescape_and_decode(&reader)?;
                match xml_tag {
                    OLIC_TAG_NAME => {
                        feature.name = value.clone();
                    }
                    OLIC_TAG_MODULE_NAME => {
                        feature.module = value.clone();
                    }
                    OLIC_TAG_USED => {
                        feature.used = value.parse()?;
                    }
                    OLIC_TAG_TOTAL => {
                        feature.total = value.parse()?;
                    }
                    OLIC_TAG_VENDOR => {
                        feature.vendor = value.clone();
                    }
                    OLIC_TAG_EXPIRATION_DATE => {
                        feature.expiration_date = value.clone();
                    }
                    OLIC_TAG_SERVER_VERSION => {
                        result.server_version = value.clone();
                    }
                    OLIC_TAG_CHECKOUTS => {
                        feature.raw_checkouts = value.clone();
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
        .register(Box::new(OLICENSE_SERVER_STATUS.clone()))
        .unwrap();

    exporter::REGISTRY
        .register(Box::new(OLICENSE_FEATURES_USED.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(OLICENSE_FEATURES_TOTAL.clone()))
        .unwrap();
}
