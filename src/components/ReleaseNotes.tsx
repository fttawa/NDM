import { Fragment } from "react";
import { api } from "../lib/api";

function inline(text: string) {
  return text.split(/(`[^`\n]+`|\*\*[^*\n]+\*\*|\[[^\]\n]+\]\(https:\/\/[^\s)]+\))/g).map((part, i) => {
    if (part.startsWith("`") && part.endsWith("`")) return <code key={i}>{part.slice(1, -1)}</code>;
    if (part.startsWith("**") && part.endsWith("**")) return <strong key={i}>{part.slice(2, -2)}</strong>;
    const link = /^\[([^\]]+)\]\((https:\/\/[^\s)]+)\)$/.exec(part);
    if (link) return <button className="about-link" key={i} onClick={() => void api.openUrl(link[2]).catch(() => {})}>{link[1]}</button>;
    return <Fragment key={i}>{part}</Fragment>;
  });
}

/** Small, React-only release notes renderer. Raw HTML is always plain text. */
export function ReleaseNotes({ text }: { text: string }) {
  return <div className="release-notes muted small">{text.split(/\r?\n/).map((line, i) => {
    const heading = /^#{1,6}\s+(.+)$/.exec(line);
    if (heading) return <p className="release-heading" key={i}>{inline(heading[1])}</p>;
    if (/^\s*[-*+]\s+/.test(line)) return <div className="release-bullet" key={i}><span aria-hidden>•</span><span>{inline(line.replace(/^\s*[-*+]\s+/, ""))}</span></div>;
    return line.trim() ? <p key={i}>{inline(line)}</p> : <br key={i} />;
  })}</div>;
}
