use std::net::ToSocketAddrs;
use std::sync::Arc;

use failure::{err_msg, Error, ResultExt};
use futures::future::FutureExt;
use http::uri::Uri;
use hyper::client::HttpConnector;
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server};
use log;
use tokio::time::timeout;

#[derive(Clone, Debug)]
pub struct ProxyParams {
    pub target_url: String,
    pub local_host: String,
    pub local_port: u16,
    pub command: Vec<String>,
}

async fn handle_request(
    params: &ProxyParams,
    client: Arc<Client<HttpConnector, Body>>,
    req: Request<Body>,
) -> Result<Response<Body>, Error> {
    let target_uri = params
        .target_url
        .parse::<Uri>()
        .context("Invalid target URL")?;

    let mut target_uri_parts = req.uri().clone().into_parts();
    target_uri_parts.scheme = target_uri.scheme().cloned();
    target_uri_parts.authority = target_uri.authority().cloned();

    let (mut request_parts, body) = req.into_parts();
    request_parts.uri = Uri::from_parts(target_uri_parts)?;

    let outgoing_request = Request::from_parts(request_parts, body);

    // Forward the request
    let result = timeout(
        std::time::Duration::from_secs(600),
        client.request(outgoing_request),
    )
    .await??;

    Ok(result)
}

pub async fn run_proxy(params: ProxyParams) -> Result<(), Error> {
    log::debug!("Running proxy with params: {:?}", params);

    // The params live for the entire duration of the program
    // and don't have any interesting destructors, so just leak them.
    let static_params: &'static ProxyParams = Box::leak(Box::new(params));

    let client_arc = Arc::new(Client::new());

    let make_service = make_service_fn(move |_: &AddrStream| {
        let per_target_client_arc = client_arc.clone();

        async move {
            let service = service_fn(move |req: Request<Body>| {
                handle_request(static_params, per_target_client_arc.clone(), req).map(|result| {
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

    let mut addrs = (&*static_params.local_host, static_params.local_port).to_socket_addrs()?;
    let addr = addrs
        .next()
        .ok_or_else(|| err_msg("Failed to resolve target address"))?;
    log::info!("Listening on {}...", addr);

    Server::bind(&addr).serve(make_service).await?;

    Ok(())
}
