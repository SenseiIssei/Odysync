/** What to know before a run that includes firmware, wherever it is started. */
export function FirmwareWarning() {
  return (
    <>
      Your selection includes firmware or BIOS updates. Keep the machine on mains power and do not
      shut it down while they run: an interrupted firmware update can leave a device unbootable.
    </>
  );
}
