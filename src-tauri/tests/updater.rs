use std::time::Duration;
use tauri_plugin_updater::UpdaterExt;

#[tokio::test]
async fn official_updater_falls_back_and_rejects_tampering_and_version_substitution() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr());
    let manifest = serde_json::json!({"version":"1.2.3","notes":"Download `.torrent` safely", "platforms": {
        "windows-x86_64": {"url": format!("{base}/payload"), "signature": include_str!("fixtures/updater/payload.bin.sig").trim()}
    }}).to_string();
    let thread = std::thread::spawn(move || {
        for _ in 0..3 {
            let request = server
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
                .unwrap();
            let response = match request.url() {
                "/fail" => tiny_http::Response::from_data(Vec::new()).with_status_code(503),
                "/manifest" => tiny_http::Response::from_data(manifest.clone().into_bytes()),
                "/payload" => tiny_http::Response::from_data(
                    include_bytes!("fixtures/updater/payload.bin").to_vec(),
                ),
                _ => panic!("Unexpected updater request"),
            };
            request.respond(response).unwrap();
        }
    });
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".into(),
        serde_json::json!({
            "pubkey":include_str!("fixtures/updater/public.key").trim(),
            "dangerousInsecureTransportProtocol":true, "requireSignedVersion":true,
        }),
    );
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap();
    let updater = app
        .updater_builder()
        .no_proxy()
        .endpoints(vec![
            format!("{base}/fail").parse().unwrap(),
            format!("{base}/manifest").parse().unwrap(),
        ])
        .unwrap()
        .build()
        .unwrap();
    let mut update = updater.check().await.unwrap().unwrap();
    assert_eq!(update.version, "1.2.3");
    let mut downloaded = 0;
    let mut payload = update
        .download(|size, _| downloaded += size, || {})
        .await
        .unwrap();
    assert_eq!(downloaded, payload.len());
    update.verify(&payload).unwrap();
    payload[0] ^= 1;
    assert!(
        update.verify(&payload).is_err(),
        "Tampered mirror payload must be rejected"
    );
    payload[0] ^= 1;
    update.version = "9.9.9".into();
    assert!(
        update.verify(&payload).is_err(),
        "A genuine old package cannot masquerade as a new version"
    );
    thread.join().unwrap();
}
