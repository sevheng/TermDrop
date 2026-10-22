//! Connecting a local shell tab. The UI sends the core a shell id from its
//! own detection, or none for the default, and never a program to run.

use gpui_kit::{Context, Window};

use super::terminal::{Status, TerminalTab, lock};

impl TerminalTab {
    pub(super) fn connect_local(
        &mut self,
        shell_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closed {
            return;
        }
        self.set_status(Status::Connecting, cx);
        let (cols, rows) = *lock(&self.size);
        let services = self.services.clone();
        let task = self.services.rt.spawn(async move {
            services
                .core
                .local_connect(
                    services.events(),
                    shell_id,
                    u32::from(cols),
                    u32::from(rows),
                )
                .await
        });
        self.pending = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task
                .await
                .unwrap_or_else(|e| Err(format!("shell task failed: {e}")));
            let _ = this.update(cx, |this, cx| {
                this.pending = None;
                match result {
                    // The shell waits up to 2 s for its output sink before
                    // it starts, so its prompt is not lost; adopting attaches
                    // it straight away.
                    Ok(id) if this.closed => this.disconnect(id),
                    Ok(id) => this.adopt_session(id, cx),
                    Err(e) => this.set_status(
                        Status::Failed(format!("Could not start a local shell: {e}")),
                        cx,
                    ),
                }
            });
        }));
    }
}
