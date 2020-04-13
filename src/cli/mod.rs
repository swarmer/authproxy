mod cmdline;

use clap::ArgMatches;
use failure::{err_msg, Error};
use tokio::runtime::Runtime;

use crate::proxy;

fn cmdline_parse_error(argname: &'static str) -> Error {
    err_msg(format!(
        "Failed to parse command line argument: {}",
        argname
    ))
}

fn get_proxy_params(matches: ArgMatches) -> Result<proxy::ProxyParams, Error> {
    log::trace!("Matches: {:?}", matches);
    Ok(proxy::ProxyParams {
        target_url: matches
            .value_of("TARGET_URL")
            .ok_or_else(|| cmdline_parse_error("TARGET_URL"))?
            .to_string(),
        insecure_https: matches.is_present("INSECURE_HTTPS"),
        local_host: matches
            .value_of("LISTEN_HOST")
            .ok_or_else(|| cmdline_parse_error("LISTEN_HOST"))?
            .to_string(),
        local_port: matches
            .value_of("LISTEN_PORT")
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| cmdline_parse_error("LISTEN_PORT"))?,
        command: matches
            .values_of("COMMAND")
            .ok_or_else(|| cmdline_parse_error("COMMAND"))?
            .map(String::from)
            .collect(),
    })
}

pub async fn cli_future() -> i32 {
    let app = cmdline::build_clap_app();
    let matches = app.get_matches();

    let result = match get_proxy_params(matches) {
        Ok(params) => proxy::run_proxy(params).await,
        Err(e) => Err(e),
    };

    match result {
        Ok(()) => 0,
        Err(error) => {
            log::error!("{}", error);
            for underlying_error in error.iter_causes() {
                log::error!("Caused by: {}", underlying_error);
            }
            1
        }
    }
}

pub fn run() -> i32 {
    env_logger::from_env(env_logger::Env::default().default_filter_or("info")).init();

    Runtime::new().unwrap().block_on(cli_future())
}
