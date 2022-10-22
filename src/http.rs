use crate::config;
use crate::constants;
use crate::exporter;

use log::{debug, error, info};
use simple_error::bail;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::time::Duration;

pub fn build_client(
    insecure_ssl: bool,
    ca_file: &str,
    timeout_sec: u64,
) -> Result<reqwest::blocking::Client, Box<dyn Error>> {
    let timeout = Duration::from_secs(timeout_sec);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "X-Clacks-Overhead",
        reqwest::header::HeaderValue::from_static("GNU Terry Pratchett"),
    );
    headers.insert(
        "Accept",
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    let mut http_client_builder = reqwest::blocking::ClientBuilder::new()
        .user_agent(constants::generate_default_user_agent())
        .default_headers(headers)
        .timeout(timeout);

    if insecure_ssl {
        http_client_builder = http_client_builder
            .danger_accept_invalid_hostnames(true)
            .danger_accept_invalid_certs(true);
    } else if !ca_file.is_empty() {
        let mut ca_buffer = Vec::new();
        let mut fd = match File::open(ca_file) {
            Ok(v) => v,
            Err(e) => bail!("can't open CA file: {}", e),
        };
        if let Err(e) = fd.read_to_end(&mut ca_buffer) {
            bail!("can't read CA data: {}", e);
        }

        let ca_cert = match reqwest::Certificate::from_pem(&ca_buffer) {
            Ok(v) => v,
            Err(e) => bail!("can't decode CA data as PEM format: {}", e),
        };

        http_client_builder = http_client_builder.add_root_certificate(ca_cert);
    }
    let http_client = match http_client_builder.build() {
        Ok(v) => v,
        Err(e) => bail!("can't create HTTP client: {}", e),
    };

    Ok(http_client)
}

pub fn get(
    http_client: &mut reqwest::blocking::Client,
    url: &str,
    user: &str,
    password: &str,
) -> Result<String, Box<dyn Error>> {
    debug!("http.rs:get: GET {}", &url);

    let response = if user.is_empty() {
        http_client.get(url).send()?
    } else {
        http_client
            .get(url)
            .basic_auth(user, Some(password))
            .send()?
    };

    if response.status() != reqwest::StatusCode::OK {
        bail!(
            "HTTP connection returned HTTP status code \"{}\" instead of \"200 OK\"",
            response.status()
        );
    }

    let reply = response.text()?;
    Ok(reply)
}

pub fn server(cfg: config::Configuration, listen_address: &str) -> Result<(), Box<dyn Error>> {
    let headers: Vec<tiny_http::Header> =
        vec![
            tiny_http::Header::from_bytes(&b"X-Clacks-Overhead"[..], &b"GNU Terry Pratchett"[..])
                .unwrap(),
        ];

    let http_server = tiny_http::Server::http(listen_address).unwrap();

    info!("http.rs:server: Listening on {}", listen_address);

    loop {
        let request = match http_server.recv() {
            Ok(v) => v,
            Err(e) => {
                error!("http.rs:server: Can't process incoming request: {}", e);
                continue;
            }
        };
        let method = request.method();
        let url = request.url();

        info!(
            "http.rs:server: HTTP {} request to {} from {:?}",
            method,
            url,
            request.remote_addr()
        );

        let status_code: tiny_http::StatusCode;
        let payload: String;

        if method == &tiny_http::Method::Get {
            match url {
                "/" => {
                    status_code = tiny_http::StatusCode::from(302_i16);
                    payload = constants::ROOT_HTML.to_string();
                }
                constants::DEFAULT_METRICS_PATH => {
                    let reply = exporter::metrics(&cfg);
                    status_code = tiny_http::StatusCode::from(200_i16);
                    payload = reply;
                }
                _ => {
                    status_code = tiny_http::StatusCode::from(404_i16);
                    payload = constants::REPLY_NOT_FOUND.to_string();
                }
            };
        } else {
            status_code = tiny_http::StatusCode::from(405_i16);
            payload = constants::REPLY_METHOD_NOT_ALLOWED.to_string();
        }

        if let Err(e) = request.respond(tiny_http::Response::new(
            status_code,
            headers.clone(),
            payload.as_bytes(),
            Some(payload.len()),
            None,
        )) {
            error!("http.rs:server: Can't send response to client: {}", e);
        }
    }
}
