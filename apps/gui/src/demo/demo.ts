/**
 * Demo answers for every command, used when the UI runs outside Tauri.
 *
 * This is what the README screenshots show and what the component tests run
 * against, so it is kept realistic: real app names, plausible versions, the
 * same shapes the Rust side returns. It never runs inside the app — `bridge`
 * imports it only when there is no Tauri IPC.
 */
import type {
  ApplyRequest,
  ApplyResultDto,
  BackendDto,
  CleanupItemDto,
  CleanupResultDto,
  ComponentStatus,
  Config,
  DeviceProblem,
  DriverBackupDto,
  HardwareInfoDto,
  HealthReportDto,
  HistoryEntryDto,
  InstalledPackageDto,
  LargeFileReport,
  ScanReport,
  ScanResult,
  StartupProgramDto,
} from "../types";

const GB = 1024 * 1024 * 1024;
const MB = 1024 * 1024;

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/**
 * What the demo has "done", so a one-click fix in the browser behaves like the
 * app: installed updates leave the list and cleaned categories measure zero.
 */
const applied = new Set<string>();
const cleaned = new Set<string>();
const schedules = new Set<string>();

const health: HealthReportDto = {
  disks: [
    {
      name: "Samsung SSD 990 PRO 2TB",
      kind: "NVMe SSD",
      size_bytes: 2000 * GB,
      state: "good",
      summary: "Healthy.",
      temperature_c: 41,
      wear_percent: 3,
      power_on_hours: 4211,
    },
    {
      name: "WD_BLACK SN850X 4TB",
      kind: "NVMe SSD",
      size_bytes: 4000 * GB,
      state: "good",
      summary: "Healthy.",
      temperature_c: 46,
      wear_percent: 9,
      power_on_hours: 9873,
    },
  ],
  battery: null,
  notes: [],
};

const backends: BackendDto[] = [
  b("direct", "Direct from publisher", true, true, false, "verified by Odysync"),
  b("windows-update", "Windows Update (security & quality)", true, true, false, "verified by the Windows Update Agent"),
  b("windows-drivers", "Windows Update (Drivers)", true, true, false, "verified by the Windows Update Agent"),
  b("windows-defender-update", "Microsoft Defender definitions", true, true, false, "verified by the Windows Update Agent"),
  b("nvidia-gpu", "NVIDIA GPU Driver", true, true, false, "verified by Odysync"),
  b("windows-feature-update", "Windows feature upgrades", true, false, true, "verified by the Windows Update Agent"),
  b("winget", "winget", true, false, true, "verified by winget"),
  b("msstore", "Microsoft Store", true, false, true, "verified by the Microsoft Store"),
  b("chocolatey", "Chocolatey", false, true, false, "verified by Chocolatey"),
  b("scoop", "Scoop", false, true, false, "verified by Scoop"),
  b("pip", "Python pip", true, true, false, "verified by pip"),
  b("npm", "Node.js npm (global)", true, true, false, "verified by npm"),
  b("cargo", "Rust cargo", true, true, false, "verified by cargo"),
  b("vscode-extension", "VS Code extensions", true, true, false, "verified by VS Code"),
];

function b(
  kind: string,
  name: string,
  available: boolean,
  enabled: boolean,
  opt_in: boolean,
  verification: string,
): BackendDto {
  return {
    kind,
    name,
    available,
    enabled,
    opt_in,
    verification,
    verification_detail: `${verification}.`,
    attended: ["windows-drivers", "nvidia-gpu", "windows-feature-update"].includes(kind),
  };
}

const scan: ScanResult = {
  actionable: [
    { backend: "direct", id: "user:vscode", name: "Visual Studio Code", installed: "1.140.0", available: "1.142.0", size_bytes: null },
    { backend: "direct", id: "user:telegram-desktop", name: "Telegram Desktop", installed: "7.2.4", available: "7.3.1", size_bytes: 59 * MB },
    { backend: "direct", id: "user:localsend", name: "LocalSend", installed: "1.17.0", available: "1.18.2", size_bytes: 18 * MB },
    { backend: "pip", id: "requests", name: "requests", installed: "2.32.3", available: "2.33.1", size_bytes: null },
    { backend: "npm", id: "typescript", name: "typescript", installed: "6.0.3", available: "7.0.2", size_bytes: null },
  ],
  skipped: [
    { backend: "nvidia-gpu", id: "geforce-driver", name: "GeForce Game Ready Driver for NVIDIA GeForce RTX 4080", reason: "requires administrator privileges", installed: "616.92", available: "617.42" },
    { backend: "direct", id: "machine:git", name: "Git for Windows", reason: "requires administrator privileges", installed: "2.56.0", available: "2.56.0.2" },
    { backend: "direct", id: "machine:notepad-plus-plus", name: "Notepad++", reason: "requires administrator privileges", installed: "8.9.7", available: "8.9.8.1" },
    { backend: "windows-update", id: "kb5101650", name: "2026-10 Cumulative Update for Windows 11 Version 25H2 (KB5101650)", reason: "requires administrator privileges", installed: "0", available: "1" },
    { backend: "windows-defender-update", id: "kb2267602", name: "Security Intelligence Update for Microsoft Defender Antivirus (KB2267602)", reason: "requires administrator privileges", installed: "0", available: "1" },
  ],
  total: 12,
  failed_backends: [],
};

const config: Config = {
  policy: { stable_only: true, require_known_versions: true, exclude: [], holds: [] },
  disabled_backends: [],
  enabled_backends: [],
  profiles: [{ name: "Work tools", packages: ["vscode", "git", "nodejs"] }],
  restore_point: true,
  scan_interval_hours: 24,
  concurrency: 4,
  proxy_url: null,
  auto_apply: false,
  notifications: true,
  max_retries: 2,
  backend_timeout_secs: 120,
} as Config;

const cleanup: CleanupItemDto[] = [
  c("temp-files", "Temporary files", "Files applications left behind. A program still using one keeps it.", 3.4 * GB, 18_420, "regenerated", true),
  c("thumbnail-cache", "Thumbnail and icon caches", "Explorer rebuilds these on demand; folders browse slower once.", 92 * MB, 31, "regenerated", true),
  c("recent-items", "Recently opened files list", "Clears the recently-opened list in Explorer and file dialogs. The files themselves are not touched; pinned items stay.", 48_000, 74, "discardable", false),
  c("recycle-bin", "Recycle Bin", "Permanently deletes everything currently in the bin or trash.", 880 * MB, 212, "loses-capability", false),
  c("error-reports", "Windows error reports", "Queued crash reports that were never sent.", 41 * MB, 12, "discardable", false),
  c("crash-dumps", "Crash dumps", "Memory dumps from past crashes. Delete only if nobody is still diagnosing one.", 0, 0, "discardable", false),
  c("windows-update-cache", "Windows Update downloads", "Already-installed update payloads. Windows re-downloads if it needs them again.", 1.2 * GB, 640, "regenerated", true),
  c("delivery-optimization", "Delivery Optimization cache", "Peer-to-peer update cache. Rebuilds itself; other machines on the network may re-download from Microsoft instead.", 310 * MB, 44, "regenerated", true),
  { ...c("browser-cache", "Browser caches", "Cached page assets only — never cookies, logins or saved passwords. Sites load slower once.", 876 * MB, 9_211, "regenerated", false), note: "Google Chrome, Microsoft Edge, Mozilla Firefox. A browser that is running is skipped." },
  c("shader-cache", "GPU shader caches", "Compiled shaders for games and apps. They are rebuilt on demand, so games may stutter briefly the first time they run afterwards.", 12.6 * GB, 832, "regenerated", false),
  { ...c("component-store", "Superseded Windows components", "Superseded component versions. Windows updates installed before this point can no longer be uninstalled.", 0, 0, "loses-capability", false), note: "Size is only known once Windows analyses the component store, which takes several minutes." },
  { ...c("previous-windows", "Previous Windows installation", "Removes the ability to roll back to the previous Windows version, permanently.", 0, 0, "loses-capability", false), report_only: true },
];

function c(
  id: string,
  label: string,
  consequence: string,
  bytes: number,
  file_count: number,
  reversibility: CleanupItemDto["reversibility"],
  selected_by_default: boolean,
): CleanupItemDto {
  return {
    id,
    label,
    consequence,
    bytes: Math.round(bytes),
    file_count,
    note: null,
    error: null,
    reversibility,
    selected_by_default,
    report_only: false,
    needs_confirmation: reversibility === "loses-capability",
  };
}

const components: ComponentStatus[] = [
  { id: "vc-redist-x64", name: "Visual C++ Redistributable (x64)", installed: "14.51.36247.0", latest: null, state: "installed", installable: true, note: "Installing fetches Microsoft's latest v14 runtime; it never replaces a newer one." },
  { id: "vc-redist-x86", name: "Visual C++ Redistributable (x86)", installed: null, latest: null, state: "missing", installable: true, note: "Installing fetches Microsoft's latest v14 runtime; it never replaces a newer one." },
  { id: "directx-legacy", name: "DirectX End-User Runtime (legacy D3DX, XInput 1.3)", installed: "June 2010", latest: "June 2010", state: "installed", installable: true, note: "Only older games need these libraries. Windows' own DirectX is not changed." },
  { id: "dotnet-windowsdesktop-8.0", name: ".NET Desktop Runtime 8.0", installed: "8.0.29", latest: "8.0.32", state: "outdated", installable: true, note: "Patches stay within .NET 8.0; Odysync never changes the major version." },
  { id: "dotnet-runtime-6.0", name: ".NET Runtime 6.0", installed: "6.0.36", latest: "6.0.36", state: "out-of-support", installable: false, note: "Microsoft no longer fixes security issues in .NET 6.0. Apps that need it keep working; consider asking their developers about a newer version." },
];

const devices: DeviceProblem[] = [
  { name: "Unknown USB Device (Device Descriptor Request Failed)", class: "USB", deviceId: "USB\\VID_0000&PID_0002\\5&2A3F", code: 43, meaning: "Windows stopped the device because it reported a problem.", suggestion: "For USB devices: try another port or cable. Otherwise update or reinstall the driver." },
];

const driverBackups: DriverBackupDto[] = [
  { id: "2026-10-12T19-04-11Z", created_at: "2026-10-12T19:04:11Z", reason: "before applying 2 driver updates", package_count: 214, size_bytes: Math.round(4.6 * GB), complete: true },
];

const history: HistoryEntryDto[] = [
  { timestamp: "2026-10-14T18:21:03Z", package: "Visual Studio Code", backend: "direct", from_version: "1.139.1", to_version: "1.140.0", status: "updated", detail: "" },
  { timestamp: "2026-10-12T19:06:44Z", package: "Realtek Audio Driver", backend: "windows-drivers", from_version: "6.0.9990.1", to_version: "6.0.9998.1", status: "updated", detail: "" },
  { timestamp: "2026-10-12T19:05:02Z", package: "2026-10 Cumulative Update (KB5101650)", backend: "windows-update", from_version: "0", to_version: "1", status: "updated", detail: "" },
  { timestamp: "2026-10-09T08:40:19Z", package: "Node.js", backend: "direct", from_version: "24.17.0", to_version: "24.18.0", status: "updated", detail: "" },
  { timestamp: "2026-10-07T21:12:55Z", package: "Telegram Desktop", backend: "direct", from_version: "7.2.3", to_version: "7.2.4", status: "failed", detail: "Telegram Desktop is running (Telegram.exe); close it and try again" },
];

const hardware: HardwareInfoDto = {
  os: "Windows 11 Pro 25H2 (build 26200)",
  machine: "Micro-Star International Co., Ltd. MS-7E12",
  cpu: "AMD Ryzen 9 9950X3D 16-Core Processor",
  cpu_threads: 32,
  memory_bytes: 64 * GB,
  gpus: [
    { name: "NVIDIA GeForce RTX 4080", vendor: "NVIDIA", driver_version: "32.0.16.1692" },
    { name: "AMD Radeon(TM) Graphics", vendor: "Advanced Micro Devices, Inc.", driver_version: "32.0.21042.62" },
  ],
  disks: [
    { mount: "C:", label: "System", filesystem: "NTFS", size_bytes: Math.round(1863 * GB), free_bytes: Math.round(612 * GB) },
    { mount: "D:", label: "Games", filesystem: "NTFS", size_bytes: Math.round(3726 * GB), free_bytes: Math.round(1410 * GB) },
  ],
};

const installed: InstalledPackageDto[] = [
  { backend: "direct", id: "user:vscode", name: "Visual Studio Code", version: "1.140.0" },
  { backend: "direct", id: "machine:git", name: "Git for Windows", version: "2.56.0" },
  { backend: "direct", id: "machine:github-cli", name: "GitHub CLI", version: "2.102.0" },
  { backend: "direct", id: "machine:nodejs", name: "Node.js", version: "24.22.0" },
  { backend: "direct", id: "machine:notepad-plus-plus", name: "Notepad++", version: "8.9.7" },
  { backend: "direct", id: "user:telegram-desktop", name: "Telegram Desktop", version: "7.2.4" },
  { backend: "pip", id: "requests", name: "requests", version: "2.32.3" },
  { backend: "npm", id: "typescript", name: "typescript", version: "6.0.3" },
];

const startup: StartupProgramDto[] = [
  { name: "Discord", command: "\"C:\\Users\\demo\\AppData\\Local\\Discord\\Update.exe\" --processStart Discord.exe", location: "HKCU\\Run", enabled: true },
  { name: "Steam", command: "\"C:\\Program Files (x86)\\Steam\\steam.exe\" -silent", location: "HKCU\\Run", enabled: false },
  { name: "OneDrive", command: "\"C:\\Program Files\\Microsoft OneDrive\\OneDrive.exe\" /background", location: "HKCU\\Run", enabled: true },
  { name: "Odysync", command: "\"C:\\Program Files\\Odysync\\odysync-gui.exe\" --minimized", location: "HKCU\\Run", enabled: true },
];

const security: ScanReport = {
  scannedAt: "2026-10-15T20:14:02Z",
  sections: [
    { name: "Microsoft Defender", ok: true, error: null, durationMs: 840 },
    { name: "Persistence", ok: true, error: null, durationMs: 2210 },
    { name: "Network", ok: true, error: null, durationMs: 610 },
    { name: "Posture", ok: true, error: null, durationMs: 390 },
  ],
  findings: [
    { id: "posture.smb1", severity: "medium", category: "posture", title: "SMBv1 client is enabled", detail: "SMBv1 is obsolete and was the vector for WannaCry. Nothing on a modern network needs it.", evidence: ["Get-WindowsOptionalFeature SMB1Protocol-Client: Enabled"], remediation: { kind: "manual", instructions: "Turn Windows features on or off → uncheck SMB 1.0/CIFS File Sharing Support." } },
    { id: "posture.uac-dimming", severity: "low", category: "posture", title: "UAC prompts do not use the secure desktop", detail: "Other programs can draw over an elevation prompt while it is shown.", evidence: ["PromptOnSecureDesktop = 0"], remediation: null },
  ],
};

const largeFiles: LargeFileReport = {
  scanned: 861_591,
  truncated: false,
  files: [
    { path: "C:\\Users\\demo\\AppData\\Local\\Docker\\wsl\\disk\\docker_data.vhdx", bytes: Math.round(32.6 * GB) },
    { path: "C:\\Users\\demo\\Videos\\Captures\\Recording 2026-09-30.mp4", bytes: Math.round(6.2 * GB) },
    { path: "C:\\Users\\demo\\Downloads\\ubuntu-26.04-desktop-amd64.iso", bytes: Math.round(5.8 * GB) },
    { path: "C:\\Users\\demo\\AppData\\Local\\pip\\cache\\http-v2\\d\\3\\c\\e\\5\\d3ce5426f3.body", bytes: Math.round(2.9 * GB) },
  ],
};

/** Answer one command the way the Rust side would. */
export async function demoInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  await wait(120);
  const answer = (value: unknown) => value as T;

  switch (command) {
    case "get_system_info":
      return answer({ os: "Windows", platform: "windows", elevated: false, version: "3.0.0", config_error: null });
    case "scan":
      await wait(500);
      return answer({
        ...scan,
        actionable: scan.actionable.filter((u) => !applied.has(`${u.backend}:${u.id}`)),
      });
    case "list_backends":
    case "refresh_backends":
      return answer(backends);
    case "get_config":
      return answer(config);
    case "save_config":
    case "hold":
    case "unhold":
    case "report_frontend_error":
    case "create_profile":
    case "delete_profile":
    case "toggle_startup_program":
    case "enable_autostart":
    case "disable_autostart":
    case "clear_update_history":
    case "reveal_path":
      return answer(undefined);
    case "apply": {
      const request = args?.request as ApplyRequest | undefined;
      await wait(1600);
      for (const u of request?.updates ?? []) applied.add(`${u.backend}:${u.id}`);
      const result: ApplyResultDto = {
        updated: request?.updates.length ?? 0,
        failed: 0,
        skipped: 0,
        reboot_required: false,
        exit_code: 0,
        entries: (request?.updates ?? []).map((u) => ({ name: u.name, status: "updated", detail: "" })),
      };
      return answer(result);
    }
    case "get_update_history":
      return answer(history);
    case "get_hardware_info":
      return answer(hardware);
    case "list_installed_packages":
      return answer(installed);
    case "get_logs":
      return answer([
        { level: "INFO", message: "scan complete backend=direct count=6", timestamp: "2026-10-15T20:12:44Z" },
        { level: "INFO", message: "installer signature verified app=vscode subject=CN=Microsoft Corporation", timestamp: "2026-10-15T20:13:31Z" },
        { level: "WARN", message: "could not read the latest release app=obs-studio error=timeout", timestamp: "2026-10-15T20:12:44Z" },
      ]);
    case "open_log_folder":
      return answer("C:\\Users\\demo\\AppData\\Roaming\\SenseiIssei\\Odysync\\data\\logs");
    case "list_profiles":
      return answer(config.profiles);
    case "get_offline_cache_status":
      return answer({ entry_count: 0, cache_size_bytes: 0 });
    case "list_offline_cache":
      return answer([]);
    case "list_startup_programs":
      return answer(startup);
    case "list_backups":
      return answer([
        { name: "Odysync: before applying 4 updates", created_at: "2026-10-12T19:03:40Z", sequence_number: 41, backup_type: "APPLICATION_INSTALL" },
      ]);
    case "is_system_protection_enabled":
      return answer(true);
    case "get_autostart":
      return answer({ enabled: true, minimized: true });
    case "security_scan":
      await wait(700);
      return answer(security);
    case "get_defender_status":
      return answer({
        real_time_protection: true,
        tamper_protection: true,
        antivirus_enabled: true,
        signature_age_days: 0,
        signature_version: "1.441.212.0",
        last_quick_scan: "2026-10-15T07:02:11Z",
        last_full_scan: "2026-10-01T03:12:40Z",
      });
    case "list_maintenance":
      return answer(["system_health"]);
    case "check_schedule":
      return answer(schedules.has(String(args?.taskName ?? args?.task_name ?? "")));
    case "create_schedule": {
      const request = args?.request as { task_name: string | null } | undefined;
      const name = request?.task_name ?? "Odysync";
      schedules.add(name);
      return answer(name);
    }
    case "remove_schedule": {
      const name = String(args?.taskName ?? args?.task_name ?? "");
      return answer(schedules.delete(name));
    }
    case "device_health":
      await wait(300);
      return answer(health);
    case "cleanup_scan":
      await wait(400);
      return answer(
        cleanup.map((item) =>
          cleaned.has(item.id) ? { ...item, bytes: 0, file_count: 0 } : item,
        ),
      );
    case "cleanup_run": {
      const ids = (args?.categories as string[]) ?? [];
      await wait(900);
      for (const id of ids) cleaned.add(id);
      const results: CleanupResultDto[] = cleanup
        .filter((item) => ids.includes(item.id))
        .map((item) => ({
          id: item.id,
          label: item.label,
          removed_bytes: Math.round(item.bytes * 0.97),
          removed_files: Math.round(item.file_count * 0.97),
          skipped: item.id === "temp-files" ? ["3 files are in use by running programs"] : [],
        }));
      return answer(results);
    }
    case "large_files":
      await wait(600);
      return answer(largeFiles);
    case "disk_optimization_plan":
      return answer([
        { drive: "C:", media_type: "SSD", operation: "Retrim" },
        { drive: "D:", media_type: "SSD", operation: "Retrim" },
      ]);
    case "components_status":
      return answer(components);
    case "component_install":
      return answer(`${String(args?.id)} installed`);
    case "device_problems":
      return answer(devices);
    case "driver_backups":
      return answer(driverBackups);
    case "driver_backup_prune":
      return answer(0);
    default:
      throw new Error(`"${command}" is not available in the browser demo`);
  }
}
