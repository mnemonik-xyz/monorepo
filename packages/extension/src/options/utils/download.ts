// Trigger a browser download for an in-memory byte buffer. Shared by
// Storage (export-as-zip) and Identity (export-keypair-as-json) sections
// so the URL.createObjectURL bookkeeping (revoke after click) lives in
// exactly one place.

export function triggerDownload(
  bytes: Uint8Array,
  filename: string,
  mimeType = "application/octet-stream",
): void {
  const blob = new Blob([bytes], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
