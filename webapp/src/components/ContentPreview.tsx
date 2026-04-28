interface ContentPreviewProps {
  content: string;
  /** Length of the underlying canonical-CBOR payload (bytes), shown alongside the text count. */
  cborByteLength?: number;
}

/**
 * Monospace preview of the bundle content the user is about to sign. Wraps
 * long lines and shows both the rendered character count and (when known) the
 * canonical-CBOR byte length, per the Risks table malicious-content row.
 */
export default function ContentPreview({
  content,
  cborByteLength,
}: ContentPreviewProps) {
  const charCount = content.length;
  // Approximate UTF-8 byte count without pulling in TextEncoder mocks during tests.
  let utf8Bytes = 0;
  try {
    utf8Bytes = new Blob([content]).size;
  } catch {
    utf8Bytes = charCount;
  }

  return (
    <section className="space-y-2" aria-label="Bundle content preview">
      <div className="flex items-center justify-between text-xs uppercase tracking-wide text-text-muted">
        <span>Content</span>
        <span aria-label="Content size in bytes">
          {utf8Bytes} bytes
          {typeof cborByteLength === "number" &&
            ` · ${cborByteLength} bytes CBOR`}
          {` · ${charCount} chars`}
        </span>
      </div>
      <pre
        className="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-md border border-text-muted/20 bg-white/5 p-4 font-mono text-sm text-text-primary"
        data-testid="content-preview"
      >
        {content}
      </pre>
    </section>
  );
}
