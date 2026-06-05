use std::path::Path;
use std::path::PathBuf;

use chrono::Utc;
use codex_protocol::ThreadId;
use codex_protocol::protocol::GitInfo;
use codex_protocol::protocol::GitSha;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionSource;
use codex_rollout::ARCHIVED_SESSIONS_SUBDIR;
use codex_rollout::append_rollout_item_to_path;
use codex_rollout::find_archived_thread_path_by_id_str;
use codex_rollout::find_thread_path_by_id_str;
use codex_rollout::read_session_meta_line;
use codex_state::ThreadMetadataBuilder;
use tracing::warn;

use super::LocalThreadStore;
use super::helpers::git_info_from_parts;
use super::live_writer;
use crate::GitInfoPatch;
use crate::ReadThreadParams;
use crate::StoredThread;
use crate::ThreadMetadataPatch;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;
use crate::UpdateThreadMetadataParams;
use crate::local::read_thread;

struct ResolvedRolloutPath {
    path: PathBuf,
    archived: bool,
}

pub(super) async fn update_thread_metadata(
    store: &LocalThreadStore,
    params: UpdateThreadMetadataParams,
) -> ThreadStoreResult<StoredThread> {
    let thread_id = params.thread_id;
    let patch = params.patch;
    if patch.is_empty() {
        return read_thread::read_thread(
            store,
            ReadThreadParams {
                thread_id,
                include_archived: params.include_archived,
                include_history: false,
            },
        )
        .await;
    }

    let needs_rollout_compat = needs_rollout_compatibility_update(&patch);
    let require_sqlite_write = sqlite_write_failure_should_block(&patch);
    let updated = apply_metadata_update(
        store,
        thread_id,
        patch.clone(),
        params.include_archived,
        require_sqlite_write,
    )
    .await?;
    if !needs_rollout_compat {
        return Ok(updated);
    }

    if live_writer::rollout_path(store, thread_id).await.is_ok() {
        live_writer::persist_thread(store, thread_id).await?;
    }
    let resolved_rollout_path =
        resolve_rollout_path(store, thread_id, params.include_archived).await?;
    let name = patch.name;
    let git_info = patch.git_info;

    let state_db_ctx = store.state_db().await;
    codex_rollout::state_db::reconcile_rollout(
        state_db_ctx.as_deref(),
        resolved_rollout_path.path.as_path(),
        store.config.default_model_provider_id.as_str(),
        None,
        &[],
        resolved_rollout_path.archived.then_some(true),
    )
    .await;

    if let Some(name) = name {
        apply_thread_name(store, thread_id, name.unwrap_or_default()).await?;
    }

    let resolved_git_info = match git_info {
        Some(git_info) => {
            let Some(state_db) = store.state_db().await else {
                return Err(ThreadStoreError::Internal {
                    message: format!("sqlite state db unavailable for thread {thread_id}"),
                });
            };
            let metadata =
                state_db
                    .get_thread(thread_id)
                    .await
                    .map_err(|err| ThreadStoreError::Internal {
                        message: format!(
                            "failed to read git metadata for thread {thread_id}: {err}"
                        ),
                    })?;
            let Some(metadata) = metadata else {
                return Err(ThreadStoreError::Internal {
                    message: format!("thread metadata unavailable before git update: {thread_id}"),
                });
            };
            let existing_git_info = git_info_from_parts(
                metadata.git_sha,
                metadata.git_branch,
                metadata.git_origin_url,
            );
            Some(resolve_git_info_patch(existing_git_info, git_info))
        }
        None => None,
    };
    if let Some((sha, branch, origin_url)) = resolved_git_info.as_ref() {
        apply_thread_git_info_to_rollout(
            resolved_rollout_path.path.as_path(),
            thread_id,
            sha,
            branch,
            origin_url,
        )
        .await?;
        apply_thread_git_info(store, thread_id, sha, branch, origin_url).await?;
    }

    let mut thread = match read_thread::read_thread(
        store,
        ReadThreadParams {
            thread_id,
            include_archived: params.include_archived,
            include_history: false,
        },
    )
    .await
    {
        Ok(thread) => thread,
        Err(_) => {
            read_thread::read_thread_by_rollout_path(
                store,
                resolved_rollout_path.path,
                params.include_archived,
                false,
            )
            .await?
        }
    };
    if let Some((sha, branch, origin_url)) = resolved_git_info {
        thread.git_info = git_info_from_parts(sha, branch, origin_url);
    }
    Ok(thread)
}

async fn apply_metadata_update(
    store: &LocalThreadStore,
    thread_id: ThreadId,
    patch: ThreadMetadataPatch,
    include_archived: bool,
    require_sqlite_write: bool,
) -> ThreadStoreResult<StoredThread> {
    let live_rollout_path = live_writer::rollout_path(store, thread_id).await.ok();
    let mut rollout_path = patch.rollout_path.clone().or(live_rollout_path);
    let mut rollout_path_archived = rollout_path
        .as_deref()
        .is_some_and(|path| rollout_path_is_archived(store, path));
    let state_db = store.state_db().await;
    let sqlite_write_result: ThreadStoreResult<()> = if let Some(state_db) = state_db.as_ref() {
        let patch = patch.clone();
        async {
            let existing =
                state_db
                    .get_thread(thread_id)
                    .await
                    .map_err(|err| ThreadStoreError::Internal {
                        message: format!("failed to read thread metadata for {thread_id}: {err}"),
                    })?;
            if existing.is_none() && rollout_path.is_none() {
                let resolved = resolve_rollout_path(store, thread_id, include_archived).await?;
                rollout_path_archived = resolved.archived;
                rollout_path = Some(resolved.path);
            }
            let mut metadata = existing.clone().unwrap_or_else(|| {
                let created_at = patch
                    .created_at
                    .or(patch.updated_at)
                    .unwrap_or_else(Utc::now);
                let mut builder = ThreadMetadataBuilder::new(
                    thread_id,
                    rollout_path.clone().unwrap_or_default(),
                    created_at,
                    patch.source.clone().unwrap_or(SessionSource::Unknown),
                );
                builder.model_provider = patch.model_provider.clone();
                builder.cwd = patch.cwd.clone().map(normalize_cwd).unwrap_or_default();
                builder.cli_version = patch.cli_version.clone();
                let mut metadata = builder.build(store.config.default_model_provider_id.as_str());
                if rollout_path_archived {
                    metadata.archived_at = Some(metadata.updated_at);
                }
                metadata
            });
            if let Some(rollout_path) = rollout_path {
                metadata.rollout_path = rollout_path;
            }
            if let Some(preview) = patch.preview {
                metadata.preview = Some(preview);
            }
            if let Some(name) = patch.name {
                metadata.title = name.unwrap_or_default();
            }
            if let Some(title) = patch.title {
                metadata.title = title;
            }
            if let Some(model_provider) = patch.model_provider {
                metadata.model_provider = model_provider;
            }
            if let Some(model) = patch.model {
                metadata.model = Some(model);
            }
            if let Some(reasoning_effort) = patch.reasoning_effort {
                metadata.reasoning_effort = Some(reasoning_effort);
            }
            if let Some(created_at) = patch.created_at {
                metadata.created_at = created_at;
            }
            if let Some(updated_at) = patch.updated_at {
                metadata.updated_at = updated_at;
            }
            if let Some(source) = patch.source {
                metadata.source = enum_to_string(&source);
            }
            if let Some(cwd) = patch.cwd {
                metadata.cwd = normalize_cwd(cwd);
            }
            if let Some(cli_version) = patch.cli_version {
                metadata.cli_version = cli_version;
            }
            if let Some(token_usage) = patch.token_usage {
                metadata.tokens_used = token_usage.total_tokens.max(0);
            }
            if let Some(first_user_message) = patch.first_user_message {
                metadata.first_user_message = Some(first_user_message);
            }
            if let Some(git_info) = patch.git_info {
                let existing_git_info = git_info_from_parts(
                    metadata.git_sha.clone(),
                    metadata.git_branch.clone(),
                    metadata.git_origin_url.clone(),
                );
                let (sha, branch, origin_url) = resolve_git_info_patch(existing_git_info, git_info);
                metadata.git_sha = sha;
                metadata.git_branch = branch;
                metadata.git_origin_url = origin_url;
            }
            state_db
                .upsert_thread(&metadata)
                .await
                .map_err(|err| ThreadStoreError::Internal {
                    message: format!("failed to update thread metadata for {thread_id}: {err}"),
                })?;
            Ok(())
        }
        .await
    } else {
        Ok(())
    };
    match (state_db.is_some(), sqlite_write_result) {
        (true, Ok(())) => {}
        (true, Err(err)) if require_sqlite_write || !sqlite_write_error_is_best_effort(&err) => {
            return Err(err);
        }
        (true, Err(err)) => {
            warn!("state db update_thread_metadata failed for {thread_id}: {err}");
        }
        (false, Ok(())) => {}
        (false, Err(err)) if require_sqlite_write || !sqlite_write_error_is_best_effort(&err) => {
            return Err(err);
        }
        (false, Err(err)) => {
            warn!("state db update_thread_metadata failed for {thread_id}: {err}");
        }
    }

    read_thread::read_thread(
        store,
        ReadThreadParams {
            thread_id,
            include_archived,
            include_history: false,
        },
    )
    .await
}

fn needs_rollout_compatibility_update(patch: &ThreadMetadataPatch) -> bool {
    if patch.name.is_some() {
        return true;
    }
    patch.git_info.is_some() && !has_observed_metadata_facts(patch)
}

fn sqlite_write_failure_should_block(patch: &ThreadMetadataPatch) -> bool {
    patch.git_info.is_some() && !has_observed_metadata_facts(patch)
}

fn sqlite_write_error_is_best_effort(err: &ThreadStoreError) -> bool {
    matches!(err, ThreadStoreError::Internal { .. })
}

fn has_observed_metadata_facts(patch: &ThreadMetadataPatch) -> bool {
    patch.rollout_path.is_some()
        || patch.preview.is_some()
        || patch.title.is_some()
        || patch.model_provider.is_some()
        || patch.model.is_some()
        || patch.reasoning_effort.is_some()
        || patch.created_at.is_some()
        || patch.source.is_some()
        || patch.cwd.is_some()
        || patch.cli_version.is_some()
        || patch.token_usage.is_some()
        || patch.first_user_message.is_some()
}

fn enum_to_string<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(value)) => value,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

fn normalize_cwd(cwd: PathBuf) -> PathBuf {
    codex_utils_path::normalize_for_path_comparison(cwd.as_path()).unwrap_or(cwd)
}

async fn apply_thread_git_info(
    store: &LocalThreadStore,
    thread_id: ThreadId,
    sha: &Option<String>,
    branch: &Option<String>,
    origin_url: &Option<String>,
) -> ThreadStoreResult<()> {
    let Some(state_db) = store.state_db().await else {
        return Err(ThreadStoreError::Internal {
            message: format!("sqlite state db unavailable for thread {thread_id}"),
        });
    };
    let updated = state_db
        .update_thread_git_info(
            thread_id,
            Some(sha.as_deref()),
            Some(branch.as_deref()),
            Some(origin_url.as_deref()),
        )
        .await
        .map_err(|err| ThreadStoreError::Internal {
            message: format!("failed to update git metadata for thread {thread_id}: {err}"),
        })?;
    if updated {
        Ok(())
    } else {
        Err(ThreadStoreError::Internal {
            message: format!("thread metadata disappeared before update completed: {thread_id}"),
        })
    }
}

fn resolve_git_info_patch(
    existing: Option<GitInfo>,
    git_info: GitInfoPatch,
) -> (Option<String>, Option<String>, Option<String>) {
    let (existing_sha, existing_branch, existing_origin_url) = match existing {
        Some(info) => (
            info.commit_hash.map(|sha| sha.0),
            info.branch,
            info.repository_url,
        ),
        None => (None, None, None),
    };
    let sha = git_info.sha.unwrap_or(existing_sha);
    let branch = git_info.branch.unwrap_or(existing_branch);
    let origin_url = git_info.origin_url.unwrap_or(existing_origin_url);
    (sha, branch, origin_url)
}

async fn apply_thread_git_info_to_rollout(
    rollout_path: &Path,
    thread_id: ThreadId,
    sha: &Option<String>,
    branch: &Option<String>,
    origin_url: &Option<String>,
) -> ThreadStoreResult<()> {
    let mut session_meta =
        read_session_meta_line(rollout_path)
            .await
            .map_err(|err| ThreadStoreError::Internal {
                message: format!("failed to set thread git metadata: {err}"),
            })?;
    if session_meta.meta.id != thread_id {
        return Err(ThreadStoreError::Internal {
            message: format!(
                "failed to set thread git metadata: rollout session metadata id mismatch: expected {thread_id}, found {}",
                session_meta.meta.id
            ),
        });
    }

    session_meta.git = Some(GitInfo {
        commit_hash: sha.as_ref().map(|sha| GitSha(sha.clone())),
        branch: branch.clone(),
        repository_url: origin_url.clone(),
    });
    append_rollout_item_to_path(rollout_path, &RolloutItem::SessionMeta(session_meta))
        .await
        .map_err(|err| ThreadStoreError::Internal {
            message: format!("failed to set thread git metadata: {err}"),
        })
}

async fn apply_thread_name(
    store: &LocalThreadStore,
    thread_id: ThreadId,
    name: String,
) -> ThreadStoreResult<()> {
    if let Some(state_db) = store.state_db().await {
        let updated = state_db
            .update_thread_title(thread_id, &name)
            .await
            .map_err(|err| ThreadStoreError::Internal {
                message: format!("failed to set thread name: {err}"),
            })?;
        if !updated {
            return Err(ThreadStoreError::Internal {
                message: format!("thread metadata unavailable before name update: {thread_id}"),
            });
        }
    }

    Ok(())
}

async fn resolve_rollout_path(
    store: &LocalThreadStore,
    thread_id: ThreadId,
    include_archived: bool,
) -> ThreadStoreResult<ResolvedRolloutPath> {
    if let Ok(path) = live_writer::rollout_path(store, thread_id).await {
        let archived = rollout_path_is_archived(store, path.as_path());
        return Ok(ResolvedRolloutPath { path, archived });
    }

    let state_db_ctx = store.state_db().await;
    let active_path = find_thread_path_by_id_str(
        store.config.codex_home.as_path(),
        &thread_id.to_string(),
        state_db_ctx.as_deref(),
    )
    .await
    .map_err(|err| ThreadStoreError::InvalidRequest {
        message: format!("failed to locate thread id {thread_id}: {err}"),
    })?;
    if let Some(path) = active_path {
        return Ok(ResolvedRolloutPath {
            path,
            archived: false,
        });
    }
    if !include_archived {
        return Err(ThreadStoreError::InvalidRequest {
            message: format!("thread not found: {thread_id}"),
        });
    }
    find_archived_thread_path_by_id_str(
        store.config.codex_home.as_path(),
        &thread_id.to_string(),
        state_db_ctx.as_deref(),
    )
    .await
    .map_err(|err| ThreadStoreError::InvalidRequest {
        message: format!("failed to locate archived thread id {thread_id}: {err}"),
    })?
    .map(|path| ResolvedRolloutPath {
        path,
        archived: true,
    })
    .ok_or_else(|| ThreadStoreError::InvalidRequest {
        message: format!("thread not found: {thread_id}"),
    })
}

fn rollout_path_is_archived(store: &LocalThreadStore, path: &Path) -> bool {
    path.starts_with(store.config.codex_home.join(ARCHIVED_SESSIONS_SUBDIR))
}
