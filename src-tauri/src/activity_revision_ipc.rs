//! Activity revision dispatch owns cancellation before Tauri queues a response.

pub(super) fn handler<R: tauri::Runtime>(
    next: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    let revision = tauri::generate_handler![activity_revision];
    move |invoke| {
        if invoke.message.command() == "activity_revision" {
            revision(invoke)
        } else {
            next(invoke)
        }
    }
}

#[tauri::command]
async fn activity_revision() -> Result<String, String> {
    let generation = crate::observability_controller::activity_revision_generation();
    tauri::async_runtime::spawn_blocking(move || {
        crate::observability_controller::activity_revision_for(generation)
    })
    .await
    .map_err(|error| format!("Activity revision task failed: {error}"))?
}

