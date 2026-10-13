#![cfg(all(feature = "desktop", feature = "test-support"))]

use conduit_lib::observability_controller;
#[path = "../src/activity_revision_ipc.rs"]
mod activity_revision_ipc;

#[tauri::command]
fn release_activity_revision() {
    observability_controller::release_activity_revision();
}

#[tauri::command]
fn passthrough() -> &'static str {
    "unchanged"
}

fn request(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    command: &str,
) -> tauri::webview::InvokeRequest {
    tauri::webview::InvokeRequest {
        cmd: command.into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: window.url().unwrap(),
        body: tauri::ipc::InvokeBody::default(),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.into(),
    }
}

fn revision(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
) -> tokio::sync::oneshot::Receiver<tauri::ipc::InvokeResponse> {
    let (send, receive) = tokio::sync::oneshot::channel();
    window.as_ref().clone().on_message(
        request(window, "activity_revision"),
        Box::new(move |_, _, response, _, _| {
            send.send(response).unwrap();
        }),
    );
    receive
}

fn value(response: tauri::ipc::InvokeResponse) -> Result<String, serde_json::Value> {
    match response {
        tauri::ipc::InvokeResponse::Ok(body) => Ok(body.deserialize().unwrap()),
        tauri::ipc::InvokeResponse::Err(tauri::ipc::InvokeError(error)) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn watches() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            std::fs::read_link(entry.path())
                .is_ok_and(|path| path.to_string_lossy().contains("inotify"))
        })
        .count()
}

// This integration binary owns the Tauri runtime. A current-thread scheduler
// leaves response tasks unpolled until after synchronous IPC release dispatch.
#[test]
fn dispatch_cancels_before_response_poll_and_preserves_new_visible_callers() {
    let data = conduit_lib::registry::DataDirTestEnv::new("activity-ipc-order");
    std::fs::write(data.dir.join("audit.jsonl"), "disposable").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    tauri::async_runtime::set(runtime.handle().clone());
    let app = tauri::test::mock_builder()
        .invoke_handler(activity_revision_ipc::handler(tauri::generate_handler![
            release_activity_revision,
            passthrough
        ]))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let release = || {
        tauri::test::get_ipc_response(&window, request(&window, "release_activity_revision"))
            .unwrap();
    };
    release();
    #[cfg(target_os = "linux")]
    let baseline = watches();

    // Release before the old response task first executes the command worker.
    let mut old = revision(&window);
    assert!(matches!(
        old.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    release();
    let result = runtime.block_on(async { value(old.await.unwrap()) });
    #[cfg(target_os = "linux")]
    let hidden_watches = watches() - baseline;
    #[cfg(target_os = "linux")]
    println!("pre-poll release: {result:?}, hidden watches={hidden_watches}");
    release(); // Clean up even when running this regression against old code.
    assert_eq!(
        result,
        Err(serde_json::json!("Activity change check cancelled"))
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        hidden_watches, 0,
        "old IPC response recreated a hidden watch"
    );

    // A new visible caller can finish before the renderer consumes old errors.
    let old = revision(&window);
    release();
    let fresh = revision(&window);
    let fresh = runtime.block_on(async { value(fresh.await.unwrap()).unwrap() });
    let old = runtime.block_on(async { value(old.await.unwrap()) });
    assert!(old.is_err());
    #[cfg(target_os = "linux")]
    assert_eq!(watches() - baseline, 1);
    let again = revision(&window);
    assert_eq!(
        runtime.block_on(async { value(again.await.unwrap()) }),
        Ok(fresh)
    );

    // A response completed before hide may reach the renderer after a fresh
    // response. Consuming it must not trigger any native cache release.
    let completed = runtime.block_on(async { revision(&window).await.unwrap() });
    release();
    let fresh = revision(&window);
    runtime.block_on(async { value(fresh.await.unwrap()).unwrap() });
    assert!(value(completed).is_ok());
    #[cfg(target_os = "linux")]
    assert_eq!(watches() - baseline, 1);
    release();
    #[cfg(target_os = "linux")]
    assert_eq!(watches(), baseline);

    // A visible read error retains one retry watch, recovers normally, and is
    // still released by the same synchronous command when the page hides.
    std::fs::remove_file(data.dir.join("audit.jsonl")).unwrap();
    std::fs::create_dir(data.dir.join("audit.jsonl")).unwrap();
    let failed = revision(&window);
    assert!(runtime
        .block_on(async { value(failed.await.unwrap()) })
        .is_err());
    std::fs::remove_dir(data.dir.join("audit.jsonl")).unwrap();
    std::fs::write(data.dir.join("audit.jsonl"), "recovered").unwrap();
    let recovered = revision(&window);
    assert!(runtime
        .block_on(async { value(recovered.await.unwrap()) })
        .is_ok());
    #[cfg(target_os = "linux")]
    assert_eq!(watches() - baseline, 1);
    release();
    #[cfg(target_os = "linux")]
    assert_eq!(watches(), baseline);

    assert_eq!(
        tauri::test::get_ipc_response(&window, request(&window, "passthrough"))
            .unwrap()
            .deserialize::<String>()
            .unwrap(),
        "unchanged"
    );
}
