use codex_config::NetworkConstraints;
use codex_config::permissions_toml::NetworkProxyConfig;
use codex_protocol::models::PermissionProfile;
use std::sync::Arc;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkProxyAuditMetadata {
    pub session_id: Option<String>,
    pub originator: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkProxySpec {
    base_config: NetworkProxyConfig,
    requirements: Option<NetworkConstraints>,
    config: NetworkProxyConfig,
}

#[derive(Debug)]
pub struct StartedNetworkProxy;

impl NetworkProxySpec {
    pub(crate) fn enabled(&self) -> bool {
        self.config.network.enabled
    }

    pub fn proxy_host_and_port(&self) -> String {
        self.config
            .network
            .proxy_url
            .strip_prefix("http://")
            .or_else(|| self.config.network.proxy_url.strip_prefix("https://"))
            .unwrap_or(&self.config.network.proxy_url)
            .to_string()
    }

    pub fn socks_enabled(&self) -> bool {
        self.config.network.enable_socks5
    }

    pub(crate) fn from_config_and_constraints(
        config: NetworkProxyConfig,
        requirements: Option<NetworkConstraints>,
        _permission_profile: &PermissionProfile,
    ) -> std::io::Result<Self> {
        Ok(Self {
            base_config: config.clone(),
            requirements,
            config,
        })
    }

    pub async fn start_proxy(
        &self,
        _permission_profile: &PermissionProfile,
        _policy_decider: Option<Arc<dyn Send + Sync>>,
        _blocked_request_observer: Option<Arc<dyn Send + Sync>>,
        _enable_network_approval_flow: bool,
        _audit_metadata: NetworkProxyAuditMetadata,
    ) -> std::io::Result<StartedNetworkProxy> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "managed network proxy has been removed",
        ))
    }

    pub(crate) fn recompute_for_permission_profile(
        &self,
        permission_profile: &PermissionProfile,
    ) -> std::io::Result<Self> {
        Self::from_config_and_constraints(
            self.base_config.clone(),
            self.requirements.clone(),
            permission_profile,
        )
    }

    pub(crate) fn with_exec_policy_network_rules<T>(&self, _exec_policy: &T) -> std::io::Result<Self> {
        Ok(self.clone())
    }

    pub(crate) async fn apply_to_started_proxy(
        &self,
        _started_proxy: &StartedNetworkProxy,
    ) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "managed network proxy has been removed",
        ))
    }
}
