import { errorKindText, t } from "@/i18n";
import type { ImportSummary } from "@/stores/importStore";

/** The lines of the result toast, in display order. */
export function summaryLines(summary: ImportSummary): string[] {
  if (summary.jobError !== null) return [t.mediaBin.result.jobFailed(errorKindText(summary.jobError))];
  const lines: string[] = [];
  if (summary.added > 0) lines.push(t.mediaBin.result.added(summary.added));
  if (summary.alreadyImported > 0) lines.push(t.mediaBin.result.alreadyImported(summary.alreadyImported));
  if (summary.failed.length > 0) lines.push(t.mediaBin.result.failed(summary.failed.length));
  if (lines.length === 0) lines.push(t.mediaBin.result.nothing);
  return lines;
}
