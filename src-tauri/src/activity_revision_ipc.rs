//! Activity revision dispatch owns cancellation before Tauri queues a response.

pub(super) fn handler<R: tauri::Runtime>(
    next: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if invoke.message.command() != "activity_revision" {
            return next(invoke);
        }
        // An async command macro calls its function inside the queued response
        // future. Capture here, synchronously at dispatch, so a subsequent hide
        // cancels this request even before that response future is first polled.
        let generation = crate::observability_controller::activity_revision_generation();
        invoke.resolver.respond_async(async move {
            tauri::async_runtime::spawn_blocking(move || {
                crate::observability_controller::activity_revision_for(generation)
            })
            .await
            .map_err(|error| format!("Activity revision task failed: {error}"))?
            .map_err(Into::into)
        });
        true
    }
}
