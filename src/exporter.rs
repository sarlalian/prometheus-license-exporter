use crate::config;
use crate::constants;
use crate::dsls;
use crate::flexlm;
use crate::hasp;
use crate::licman20;
use crate::lmx;
use crate::olicense;
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

    if let Some(lmx) = &cfg.lmx {
        if !lmx.is_empty() {
            lmx::register()
        }
    }

    if let Some(dsls) = &cfg.dsls {
        if !dsls.is_empty() {
            dsls::register()
        }
    }

    if let Some(licman20) = &cfg.licman20 {
        if !licman20.is_empty() {
            licman20::register()
        }
    }

    if let Some(hasp) = &cfg.hasp {
        if !hasp.is_empty() {
            hasp::register();
        }
    }

    if let Some(olicense) = &cfg.olicense {
        if !olicense.is_empty() {
            olicense::register()
        }
    }
}

pub fn metrics(cfg: &config::Configuration) -> String {
    let encoder = TextEncoder::new();
    let mut buffer = String::new();

    if let Some(flexlm) = &cfg.flexlm {
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

    if let Some(lmx) = &cfg.lmx {
        let mut lmxendutil = constants::DEFAULT_LMXENDUTIL.to_string();
        if let Some(glob) = cfg.global.clone() {
            if let Some(_lmxendutil) = glob.lmxendutil {
                lmxendutil = _lmxendutil;
            }
        }

        for _lmx in lmx {
            match lmx::fetch(_lmx, &lmxendutil) {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        "Can't fetch LM-X license information for {}: {}",
                        _lmx.name, e
                    );
                }
            };
        }
    }

    if let Some(dsls) = &cfg.dsls {
        let mut dslicsrv = constants::DEFAULT_DSLICSRV.to_string();
        if let Some(glob) = cfg.global.clone() {
            if let Some(_dslicsrv) = glob.dslicsrv {
                dslicsrv = _dslicsrv;
            }
        }

        for _dsls in dsls {
            match dsls::fetch(_dsls, &dslicsrv) {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        "Can't fetch DSLS license information for {}: {}",
                        _dsls.name, e
                    );
                }
            };
        }
    }

    if let Some(licman20) = &cfg.licman20 {
        let mut licman20_appl = constants::DEFAULT_LICMAN20_APPL.to_string();
        if let Some(glob) = cfg.global.clone() {
            if let Some(_licman20_appl) = glob.licman20_appl {
                licman20_appl = _licman20_appl;
            }
        }

        for _licman20 in licman20 {
            match licman20::fetch(_licman20, &licman20_appl) {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        "Can't fetch Licman20 license information for {}: {}",
                        _licman20.name, e
                    );
                }
            };
        }
    }

    if let Some(hasp) = &cfg.hasp {
        for _hasp in hasp {
            match hasp::fetch(_hasp) {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        "Can't fetch HASP license information for {}: {}",
                        _hasp.name, e
                    );
                }
            };
        }
    }

    if let Some(olicense) = &cfg.olicense {
        for _olic in olicense {
            match olicense::fetch(_olic) {
                Ok(_) => {}
                Err(e) => {
                    error!(
                        "Can't fetch OLicense license information for {}: {}",
                        _olic.name, e
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
