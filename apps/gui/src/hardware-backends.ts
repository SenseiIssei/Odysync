/**
 * Which backend kinds represent hardware rather than ordinary software.
 *
 * These ids come from `BackendKind::id()` in `odysync-core`; keep them in step
 * with `backend_kind_from_str` in the Tauri command layer.
 */

export const DRIVER_BACKENDS = new Set([
  "windows-drivers",
  "nvidia-gpu",
  "amd-gpu",
  "intel-gpu",
  "qualcomm-gpu",
  "nvidia-geforce-experience",
  "intel-dsa",
]);

export const FIRMWARE_BACKENDS = new Set([
  "dell-firmware",
  "hp-firmware",
  "lenovo-firmware",
  "fwupd",
  "mac-firmware",
]);

export const OEM_BACKENDS = new Set([
  "dell-command-update",
  "hp-image-assistant",
  "lenovo-system-update",
  "msi-center",
  "asus-armoury",
  "gigabyte-control-center",
  "acer-care-center",
  "razer-synapse",
  "virtualization-guest",
]);

export type HardwareGroup = "driver" | "firmware" | "oem";

export function hardwareGroupOf(backend: string): HardwareGroup | null {
  if (DRIVER_BACKENDS.has(backend)) return "driver";
  if (FIRMWARE_BACKENDS.has(backend)) return "firmware";
  if (OEM_BACKENDS.has(backend)) return "oem";
  return null;
}

export function isHardwareBackend(backend: string): boolean {
  return hardwareGroupOf(backend) !== null;
}

export const GROUP_LABELS: Record<HardwareGroup, string> = {
  driver: "Drivers",
  firmware: "Firmware & BIOS",
  oem: "Vendor update tools",
};

export const GROUP_HINTS: Record<HardwareGroup, string> = {
  driver: "Graphics, chipset and device drivers.",
  firmware:
    "Firmware and BIOS updates. These carry the most risk of the lot — do not interrupt one, and run it on mains power.",
  oem: "Updates delivered through your machine vendor's own tool.",
};
