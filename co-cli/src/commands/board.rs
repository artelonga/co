use co_web::baseline;
use co_web::config::WebConfig;
use co_web::server::start_server;

pub fn run(port: u16, data: String, static_dir: String, default_variant: String) {
    let universo_dir = format!("{}/universes", data);
    let config = WebConfig {
        port,
        data_dir: data,
        static_dir,
        default_variant,
        experiments: true,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir,
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: std::env::var("CO_ENV").unwrap_or_else(|_| "prod".into()),
        wae_endpoint: std::env::var("WAE_ENDPOINT").ok(),
        wae_api_key: std::env::var("WAE_API_KEY").ok(),
        cookie_domain: std::env::var("CO_COOKIE_DOMAIN").ok(),
        quilombo_legacy_login: true,
    };

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(start_server(config));
}

pub fn export(data_dir: &str, dest_dir: &str) {
    match baseline::export_data(data_dir, dest_dir) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Export failed: {}", e);
            std::process::exit(1);
        }
    }
}

pub fn reset(data_dir: &str, confirm: bool) {
    if !confirm {
        eprintln!("This will discard all local edits and restore baseline data.");
        eprintln!("Run with --confirm to proceed.");
        std::process::exit(1);
    }
    baseline::reset_to_baseline(data_dir);
    println!("Data reset to embedded baseline.");
}
