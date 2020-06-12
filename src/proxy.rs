use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use failure::{err_msg, Error, ResultExt};
use futures::future::FutureExt;
use http::header::HeaderValue;
use http::uri::Uri;
use hyper::client::HttpConnector;
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server};
use hyper_tls::HttpsConnector;
use native_tls::TlsConnector;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

#[derive(Debug)]
pub struct ProxyParams {
    pub target_url: String,
    pub insecure_https: bool,
    pub local_host: String,
    pub local_port: u16,
    pub cache_ttl_secs: u64,
    pub command: Vec<String>,
}

#[derive(Clone, Debug)]
struct TokenCacheEntry {
    token: String,
    inserted_at: Instant,
}

impl TokenCacheEntry {
    fn new(token: String) -> Self {
        TokenCacheEntry {
            token,
            inserted_at: Instant::now(),
        }
    }
}

#[derive(Debug)]
struct TokenCache {
    ttl: Duration,
    entry: Mutex<Option<TokenCacheEntry>>,
}

impl TokenCache {
    fn new(ttl: Duration) -> Self {
        TokenCache {
            ttl,
            entry: Mutex::new(None),
        }
    }

    async fn get_or_refresh<C, F>(&self, callback: C) -> Result<String, Error>
    where
        C: FnOnce() -> F,
        F: std::future::Future<Output = Result<String, Error>>,
    {
        let mut entry_guard = self.entry.lock().await;

        let entry = entry_guard
            .as_ref()
            .filter(|entry| entry.inserted_at + self.ttl > Instant::now());

        match entry {
            Some(entry) => Ok(entry.token.clone()),
            None => {
                let token = callback().await?;
                *entry_guard = Some(TokenCacheEntry::new(token.clone()));
                Ok(token)
            }
        }
    }
}

#[derive(Debug)]
pub struct ProxyContext {
    params: ProxyParams,
    cache: TokenCache,
}

impl ProxyContext {
    pub fn new(params: ProxyParams) -> Self {
        ProxyContext {
            cache: TokenCache::new(Duration::from_secs(params.cache_ttl_secs)),
            params,
        }
    }
}

async fn handle_request(
    ctx: &ProxyContext,
    client: Arc<Client<HttpsConnector<HttpConnector>, Body>>,
    req: Request<Body>,
) -> Result<Response<Body>, Error> {
    let target_uri = ctx
        .params
        .target_url
        .parse::<Uri>()
        .context("Invalid target URL")?;

    let mut target_uri_parts = req.uri().clone().into_parts();
    target_uri_parts.scheme = target_uri.scheme().cloned();
    target_uri_parts.authority = target_uri.authority().cloned();

    let (mut request_parts, body) = req.into_parts();
    request_parts.uri = Uri::from_parts(target_uri_parts)?;

    let token_value = ctx
        .cache
        .get_or_refresh(|| async {
            log::debug!("Running the command to obtain the authorization header");
            let output = Command::new(ctx.params.command[0].clone())
                .args(ctx.params.command[1..].iter().map(Clone::clone))
                .output()
                .await?;

            if !output.status.success() {
                return Err(err_msg(format!(
                    "Failed to obtain the header value, subprocess result: {:?}",
                    output
                )));
            }

            Ok(String::from_utf8(output.stdout)?.trim().to_string())
        })
        .await?;

    let token_header = format!("Bearer {}", token_value);
    log::debug!("Will use token: `{}`", token_header);
    request_parts
        .headers
        .insert("Authorization", HeaderValue::from_str(&token_header)?);

    // The incoming host header will very likely be considered incorrect by the target server
    request_parts.headers.remove("Host");

    let outgoing_request = Request::from_parts(request_parts, body);

    // Forward the request
    let result = timeout(Duration::from_secs(600), client.request(outgoing_request)).await??;

    Ok(result)
}

fn get_https_client(
    params: &ProxyParams,
) -> Result<Client<HttpsConnector<HttpConnector>, Body>, Error> {
    let tls_connector = tokio_tls::TlsConnector::from(
        TlsConnector::builder()
            .danger_accept_invalid_certs(params.insecure_https)
            .build()?,
    );

    let mut http_connector = HttpConnector::new();
    http_connector.enforce_http(false);
    let https_connector = HttpsConnector::from((http_connector, tls_connector));
    Ok(Client::builder().build::<HttpsConnector<HttpConnector>, hyper::Body>(https_connector))
}

pub async fn run_proxy(ctx: ProxyContext) -> Result<(), Error> {
    log::debug!("Running proxy with params: {:?}", ctx.params);

    // The params live for the entire duration of the program
    // and don't have any interesting destructors, so just leak them.
    let ctx: &'static ProxyContext = Box::leak(Box::new(ctx));

    let client_arc = Arc::new(get_https_client(&ctx.params)?);

    let make_service = make_service_fn(move |_: &AddrStream| {
        let per_target_client_arc = client_arc.clone();

        async move {
            let service = service_fn(move |req: Request<Body>| {
                handle_request(ctx, per_target_client_arc.clone(), req).map(|result| {
                    if let Err(ref err) = result {
                        log::error!("{}", err);
                        for underlying_error in err.iter_causes() {
                            log::error!("Caused by: {}", underlying_error);
                        }
                    }

                    result
                })
            });

            Ok::<_, hyper::Error>(service)
        }
    });

    let mut addrs = (&*ctx.params.local_host, ctx.params.local_port).to_socket_addrs()?;
    let addr = addrs
        .next()
        .ok_or_else(|| err_msg("Failed to resolve target address"))?;
    log::info!("Listening on {}...", addr);

    Server::bind(&addr).serve(make_service).await?;

    Ok(())
}
