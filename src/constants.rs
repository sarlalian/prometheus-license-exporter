pub const NAME: &str = env!("CARGO_PKG_NAME");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const VERSION: &str = "1.5.2";

pub const DEFAULT_TIMEOUT: u64 = 60;
pub const DEFAULT_PROMETHEUS_ADDRESS: &str = "localhost:9998";

pub const DEFAULT_LMUTIL: &str = "lmutil";
pub const DEFAULT_RLMUTIL: &str = "rlmutil";
pub const DEFAULT_LMXENDUTIL: &str = "lmxendutil";
pub const DEFAULT_DSLICSRV: &str = "dslicsrv";
pub const DEFAULT_LICMAN20_APPL: &str = "licman20_appl";
pub const DEFAULT_HASP_PORT: &str = "1947";
pub const DEFAULT_METRICS_PATH: &str = "/metrics";

pub const ROOT_HTML: &str = "<html>\n<head><title>License exporter</title></head>\n<body>\n<h1>License exporter</h1>\n<p><a href=\"/metric\">Metrics</a></p>\n</body>\n</html>\n";

pub const REPLY_METHOD_NOT_ALLOWED: &str = "Method not allowed";
pub const REPLY_NOT_FOUND: &str = "Not found";

pub fn generate_default_user_agent() -> String {
    format!("{}/{} ({})", NAME, VERSION, SOURCE)
}
