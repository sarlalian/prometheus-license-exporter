use crate::config;
use crate::constants;
use crate::flexlm;
use crate::rlm;

use lazy_static::lazy_static;
use log::error;
use prometheus::{Registry, TextEncoder};

// Global registry
lazy_static! {
    pub static ref REGISTRY: Registry = Registry::new();
}

pub fn register(cfg: &config::Configuration) {
    if let Some(flexlm) = &cfg.flexlm {
        if !flexlm.is_empty() {
            flexlm::register()
        }
    }

    if let Some(rlm) = &cfg.rlm {
        if !rlm.is_empty() {
            rlm::register()
        }
    }
}

pub fn metrics(cfg: &config::Configuration) -> String {
    let encoder = TextEncoder::new();
    let mut buffer = String::new();

    if let Some(flexlm) = &cfg.flexlm {
        // XXX: It's not neccessary to get lmutil every time ...
        let mut lmutil = constants::DEFAULT_LMUTIL.to_string();
        if let Some(glob) = cfg.global.clone() {
            if let Some(_lmutil) = glob.lmutil {
                lmutil = _lmutil;
            }
        }

        for flex in flexlm {
            match flexlm::fetch(flex, &lmutil) {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        "Can't fetch FlexLM license information for {}: {}",
                        flex.name, e
                    );
                }
            };
        }
    }

    if let Some(rlm) = &cfg.rlm {
        // XXX: It's not neccessary to get lmutil every time ...
        let mut rlmutil = constants::DEFAULT_RLMUTIL.to_string();
        if let Some(glob) = cfg.global.clone() {
            if let Some(_rlmutil) = glob.rlmutil {
                rlmutil = _rlmutil;
            }
        }

        for _rlm in rlm {
            match rlm::fetch(_rlm, &rlmutil) {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        "Can't fetch RLM license information for {}: {}",
                        _rlm.name, e
                    );
                }
            };
        }
    }

    if let Err(e) = encoder.encode_utf8(&REGISTRY.gather(), &mut buffer) {
        error!("Can't encode metrics as UTF8 string: {}", e);
    }

    if let Err(e) = encoder.encode_utf8(&prometheus::gather(), &mut buffer) {
        error!("Can't encode metrics as UTF8 string: {}", e);
    };
    buffer
}
