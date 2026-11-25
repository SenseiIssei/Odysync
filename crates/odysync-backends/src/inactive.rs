//! Sources Odysync knows by name and does not use.
//!
//! Each of these used to be listed as a working source. None could keep the
//! contract: the tool installs a set instead of one approved item, has no
//! command line at all, or nobody publishes the version to compare with. A
//! source that cannot work must not look like one that found nothing, so they
//! are never available, never listed, and refuse to install if called anyway.
//! The kinds stay in the model because configuration files name them.
//!
//! One returns from here when it can install exactly one approved item and
//! read the result back; see ROADMAP.md.

/// A backend that exists in the model but cannot keep the contract: its tool
/// cannot install one exact item, or nobody publishes the version to compare
/// with. It is never available, so it is never listed as an update source, and
/// it refuses to install anything if it is called anyway.
macro_rules! inactive_backend {
    ($(#[$meta:meta])* $name:ident, $kind:ident, $display:literal, $why:literal) => {
        $(#[$meta])*
        pub struct $name;

        impl $name {
            pub fn new() -> Self {
                Self
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        #[async_trait::async_trait]
        impl odysync_core::backend::Backend for $name {
            fn kind(&self) -> odysync_core::model::BackendKind {
                odysync_core::model::BackendKind::$kind
            }

            fn display_name(&self) -> &str {
                $display
            }

            async fn is_available(&self) -> bool {
                false
            }

            async fn scan(
                &self,
            ) -> odysync_core::error::Result<Vec<odysync_core::model::UpdateCandidate>> {
                Ok(Vec::new())
            }

            async fn apply(
                &self,
                _candidate: &odysync_core::model::UpdateCandidate,
            ) -> odysync_core::error::Result<()> {
                Err(odysync_core::error::Error::unavailable($display, $why))
            }

            async fn installed_version(
                &self,
                _candidate: &odysync_core::model::UpdateCandidate,
            ) -> odysync_core::error::Result<Option<String>> {
                Ok(None)
            }
        }
    };
}

inactive_backend!(
    /// .NET global tools.
    ///
    /// `dotnet tool list` names neither the newest version nor the feed a tool
    /// came from. Asking nuget.org by name would offer a public package as the
    /// update of a private tool of the same name.
    DotnetToolBackend,
    DotnetTool,
    ".NET global tools",
    "the .NET command line does not say which version is newest or where a tool came from; \
     update tools with `dotnet tool update -g <id>`"
);

inactive_backend!(
    /// VS Code extensions.
    ///
    /// The `code` command lists installed versions only, and VS Code updates
    /// its extensions itself.
    VscodeExtensionBackend,
    VscodeExtension,
    "VS Code extensions",
    "VS Code updates its extensions itself and offers no way to ask for the newest version"
);

inactive_backend!(
    /// JetBrains plugins: updated from within the IDE, which is the only place
    /// that knows which plugin version fits its build.
    JetbrainsPluginBackend,
    JetbrainsPlugin,
    "JetBrains IDE plugins",
    "JetBrains plugins must be updated from within the IDE"
);

inactive_backend!(
    /// Windows optional features have no versions, so there is never an update
    /// to compare. Enabling a feature is not an update either.
    WindowsOptionalFeatureBackend,
    WindowsOptionalFeature,
    "Windows optional features",
    "optional features have no versions; Windows Update services them"
);

inactive_backend!(
    /// GeForce Experience has no command line. The NVIDIA driver source reads
    /// NVIDIA's own feed instead.
    NvidiaGeForceExperienceBackend,
    NvidiaGeForceExperience,
    "NVIDIA GeForce Experience",
    "GeForce Experience has no command line; NVIDIA drivers come from the NVIDIA GPU source"
);

inactive_backend!(
    /// Intel Driver & Support Assistant has no command line.
    IntelDsaBackend,
    IntelDsa,
    "Intel Driver & Support Assistant",
    "Intel DSA has no command line; open the assistant to update Intel drivers"
);

inactive_backend!(
    /// Updated by Acer Care Center itself.
    AcerCareCenterBackend,
    AcerCareCenter,
    "Acer Care Center",
    "Acer Care Center has no command line; open it to update Acer software"
);

inactive_backend!(
    /// Updated by Armoury Crate itself.
    AsusArmouryBackend,
    AsusArmoury,
    "ASUS Armoury Crate",
    "Armoury Crate has no command line; open it to update ASUS software"
);

inactive_backend!(
    /// Installs everything that is pending of a kind, so it is not used.
    DellCommandUpdateBackend,
    DellCommandUpdate,
    "Dell Command Update",
    "dcu-cli cannot install one exact update; open Dell Command | Update"
);

inactive_backend!(
    /// Updated by Gigabyte Control Center itself.
    GigabyteControlCenterBackend,
    GigabyteControlCenter,
    "Gigabyte Control Center",
    "Gigabyte Control Center has no command line; open it to update Gigabyte software"
);

inactive_backend!(
    /// Installs everything it recommends, so it is not used.
    HpImageAssistantBackend,
    HpImageAssistant,
    "HP Image Assistant",
    "HP Image Assistant cannot install one exact update from here; open it instead"
);

inactive_backend!(
    /// Installs everything it finds, so it is not used.
    LenovoSystemUpdateBackend,
    LenovoSystemUpdate,
    "Lenovo System Update",
    "Lenovo System Update cannot install one exact update; use Lenovo Vantage"
);

inactive_backend!(
    /// Updated by MSI Center itself.
    MsiCenterBackend,
    MsiCenter,
    "MSI Center",
    "MSI Center has no command line; use its Live Update page"
);

inactive_backend!(
    /// Updated by Razer Synapse itself.
    RazerSynapseBackend,
    RazerSynapse,
    "Razer Synapse",
    "Razer Synapse has no command line and updates itself"
);

inactive_backend!(
    /// `dcu-cli /applyUpdates` filters by type and severity only.
    /// <https://www.dell.com/support/manuals/en-us/command-update/dcu_rg/dell-command-update-cli-commands>
    DellFirmwareBackend,
    DellFirmware,
    "Dell firmware (dcu-cli)",
    "dcu-cli cannot install one exact firmware update; use Dell Command | Update"
);

inactive_backend!(
    /// HP Image Assistant installs what its analysis selects.
    HpFirmwareBackend,
    HpFirmware,
    "HP firmware (HPIA)",
    "HP Image Assistant cannot install one exact firmware update from here; open it instead"
);

inactive_backend!(
    /// Lenovo System Update installs what its search finds.
    LenovoFirmwareBackend,
    LenovoFirmware,
    "Lenovo firmware (SUHelper)",
    "Lenovo System Update cannot install one exact firmware update; use Lenovo Vantage"
);

inactive_backend!(
    /// There is no source that could be asked for an AMD driver.
    AmdGpuBackend,
    AmdGpu,
    "AMD GPU Driver",
    "AMD offers no way to look up or install one driver version; use AMD Software or Windows \
     Update"
);

inactive_backend!(
    /// There is no source that could be asked for an Intel driver.
    IntelGpuBackend,
    IntelGpu,
    "Intel GPU Driver",
    "Intel graphics drivers are not available through winget; use Intel Driver & Support \
     Assistant or Windows Update"
);

inactive_backend!(
    /// There is no source that could be asked for a Qualcomm driver.
    QualcommGpuBackend,
    QualcommGpu,
    "Qualcomm Adreno GPU",
    "Qualcomm drivers are not available through winget; they come with Windows Update"
);

inactive_backend!(
    /// Guest tools are updated from the hypervisor they belong to.
    VirtualizationGuestBackend,
    VirtualizationGuest,
    "Virtualization Guest Tools",
    "guest tools have to match the hypervisor; update them from VirtualBox, VMware or the \
     VirtIO image"
);

inactive_backend!(
    /// AppImages cannot be compared or pinned from the outside.
    AppImageBackend,
    AppImage,
    "AppImage",
    "an AppImage does not say which version it is or which one is newest; update it with the \
     tool it came from"
);

#[cfg(test)]
mod tests {
    use super::*;
    use odysync_core::backend::Backend;
    use odysync_core::error::Error;
    use odysync_core::model::{PackageId, UpdateCandidate};
    use odysync_core::version::Version;

    #[tokio::test]
    async fn none_of_them_is_listed_or_installs_anything() {
        let all: Vec<Box<dyn Backend>> = vec![
            Box::new(DotnetToolBackend::new()),
            Box::new(VscodeExtensionBackend::new()),
            Box::new(JetbrainsPluginBackend::new()),
            Box::new(WindowsOptionalFeatureBackend::new()),
            Box::new(NvidiaGeForceExperienceBackend::new()),
            Box::new(IntelDsaBackend::new()),
            Box::new(AcerCareCenterBackend::new()),
            Box::new(AsusArmouryBackend::new()),
            Box::new(DellCommandUpdateBackend::new()),
            Box::new(GigabyteControlCenterBackend::new()),
            Box::new(HpImageAssistantBackend::new()),
            Box::new(LenovoSystemUpdateBackend::new()),
            Box::new(MsiCenterBackend::new()),
            Box::new(RazerSynapseBackend::new()),
            Box::new(DellFirmwareBackend::new()),
            Box::new(HpFirmwareBackend::new()),
            Box::new(LenovoFirmwareBackend::new()),
            Box::new(AmdGpuBackend::new()),
            Box::new(IntelGpuBackend::new()),
            Box::new(QualcommGpuBackend::new()),
            Box::new(VirtualizationGuestBackend::new()),
            Box::new(AppImageBackend::new()),
        ];
        for backend in all {
            let candidate = UpdateCandidate {
                id: PackageId::new(backend.kind(), "anything"),
                name: "Anything".into(),
                installed: Version::parse("1.0.0"),
                available: Version::parse("2.0.0"),
                size_bytes: None,
                expected_sha256: None,
            };
            let name = backend.display_name().to_string();
            assert!(!name.is_empty());
            assert!(!backend.is_available().await, "{name}");
            assert!(backend.scan().await.unwrap().is_empty(), "{name}");
            assert!(
                matches!(
                    backend.apply(&candidate).await.unwrap_err(),
                    Error::BackendUnavailable { .. }
                ),
                "{name}"
            );
            assert_eq!(
                backend.installed_version(&candidate).await.unwrap(),
                None,
                "{name}"
            );
        }
    }
}
