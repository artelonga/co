use co_web::config::WebConfig;
use co_web::server::start_server_on;

pub fn run(port: u16, data_dir: String, public: bool, open: bool) {
    let bind_host = if public {
        eprintln!("Warning: --public binds to 0.0.0.0 — this exposes CO on your local network.");
        eprintln!("  Anyone on your network can reach this server.");
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };

    let url = format!("http://127.0.0.1:{}", port);
    let universo_dir = format!("{}/universes", data_dir);

    let config = WebConfig {
        port,
        data_dir,
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: true,
        plugins_dir: "plugins".to_string(),
        game_db_path: std::env::var("GAME_DB_PATH").ok(),
        universo_dir,
        gestao_github_admins: vec![],
        universe_key: None,
        // CO-309: `co serve` is by definition local. Default to "local" so
        // uat-login / admin login modal / dev-friendly defaults are on out of
        // the box. Deployed `co-web` still defaults to "prod" via WebConfig::from(Args).
        co_env: std::env::var("CO_ENV").unwrap_or_else(|_| "local".into()),
        wae_endpoint: std::env::var("WAE_ENDPOINT").ok(),
        wae_api_key: std::env::var("WAE_API_KEY").ok(),
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
    };

    eprintln!("✓ co {} listening on {}", env!("CARGO_PKG_VERSION"), url);

    if open {
        eprintln!("✓ Opening browser...");
        launch_browser(&url);
    }

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(start_server_on(config, bind_host));
}

fn launch_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();

    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn();

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let _ = url;
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_default_port() {
        assert_eq!(54321_u16, 54321);
    }
}
