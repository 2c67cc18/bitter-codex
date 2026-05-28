use super::*;

pub(crate) fn thread_settings_from_config_snapshot(
    config_snapshot: &ThreadConfigSnapshot,
) -> ThreadSettings {
    ThreadSettings {
        cwd: config_snapshot.cwd.clone(),
        model: config_snapshot.model.clone(),
        model_provider: config_snapshot.model_provider_id.clone(),
        service_tier: config_snapshot.service_tier.clone(),
        effort: config_snapshot.reasoning_effort,
        summary: config_snapshot.reasoning_summary,
    }
}

pub(crate) fn thread_settings_from_core_snapshot(
    snapshot: codex_protocol::protocol::ThreadSettingsSnapshot,
) -> ThreadSettings {
    ThreadSettings {
        cwd: snapshot.cwd,
        model: snapshot.model,
        model_provider: snapshot.model_provider_id,
        service_tier: snapshot.service_tier,
        effort: snapshot.reasoning_effort,
        summary: snapshot.reasoning_summary,
    }
}

pub(super) fn thread_started_notification(mut thread: Thread) -> ThreadStartedNotification {
    thread.turns.clear();
    ThreadStartedNotification { thread }
}
