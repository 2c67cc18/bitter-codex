mod auto_compact_window;
mod service;
mod session;
mod turn;

use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::FileSystemPermissions;

pub(crate) use auto_compact_window::AutoCompactWindowSnapshot;
pub(crate) use service::SessionServices;
pub(crate) use session::SessionState;
pub(crate) use turn::ActiveTurn;
pub(crate) use turn::MailboxDeliveryPhase;
pub(crate) use turn::RunningTask;
pub(crate) use turn::TaskKind;
pub(crate) use turn::TurnState;

fn merge_granted_permission_profiles(
    previous: Option<&AdditionalPermissionProfile>,
    grant: &AdditionalPermissionProfile,
) -> Option<AdditionalPermissionProfile> {
    let mut merged = previous.cloned().unwrap_or_default();

    if let Some(network) = &grant.network {
        merged.network = Some(network.clone());
    }

    if let Some(file_system) = &grant.file_system {
        merged.file_system = Some(match merged.file_system.take() {
            Some(previous_file_system) => merge_file_system_permissions(
                previous_file_system,
                file_system.clone(),
            ),
            None => file_system.clone(),
        });
    }

    (!merged.is_empty()).then_some(merged)
}

fn merge_file_system_permissions(
    mut previous: FileSystemPermissions,
    grant: FileSystemPermissions,
) -> FileSystemPermissions {
    for entry in grant.entries {
        if !previous.entries.contains(&entry) {
            previous.entries.push(entry);
        }
    }
    if grant.glob_scan_max_depth.is_some() {
        previous.glob_scan_max_depth = grant.glob_scan_max_depth;
    }
    previous
}
