use clap::{App, AppSettings, Arg};

pub fn build_clap_app() -> App<'static, 'static> {
    App::new("authproxy")
        .version(crate::VERSION)
        .author("Author: Anton Barkovsky")
        .about("A Proxy that injects the Authorization header")
        .setting(AppSettings::TrailingVarArg)
        .arg(
            Arg::with_name("TARGET_URL")
                .required(true)
                .help("Target URL"),
        )
        .arg(
            Arg::with_name("LISTEN_HOST")
                .short("h")
                .long("listen-host")
                .takes_value(true)
                .value_name("LISTEN_HOST")
                .default_value("127.0.0.1")
                .help("Which host to listen on"),
        )
        .arg(
            Arg::with_name("LISTEN_PORT")
                .short("p")
                .long("listen-port")
                .takes_value(true)
                .value_name("LISTEN_PORT")
                .default_value("4545")
                .validator(|s| {
                    s.parse::<u16>()
                        .and(Ok(()))
                        .or_else(|_| Err(String::from("Invalid port")))
                })
                .help("Which port to listen on"),
        )
        .arg(
            Arg::with_name("COMMAND")
                .multiple(true)
                .required(true)
                .help(concat!(
                    "Command that will be ran for every request and will output",
                    " Authorization header value",
                )),
        )
}
