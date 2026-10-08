import type { Metadata } from "next";
import { SessionRefresh } from "@/components/session-refresh";
import { authConfigured, getAuth0 } from "@/lib/auth0";
import { isAllowed, SESSION_SECONDS } from "@/lib/policy";
import "./globals.css";

export const metadata: Metadata = {
  title: "Backchannel · Viewer",
  description: "A private, read-only view of Backchannel.",
  robots: { index: false, follow: false },
};

export default async function Layout({ children }: { children: React.ReactNode }) {
  const session = authConfigured() ? await getAuth0().getSession() : null;
  const expiresAt = isAllowed(session, process.env.ALLOWED_EMAILS)
    ? (session!.internal.createdAt + SESSION_SECONDS) * 1000
    : undefined;

  return (
    <html lang="en">
      <body>
        <SessionRefresh expiresAt={expiresAt} />
        <a className="skip" href="#content">Skip to content</a>
        <div className="shell">
          <aside>
            <a className="brand" href="/"><span className="brand-mark">b</span>backchannel</a>
            <span className="readonly">READ-ONLY VIEWER</span>
            <nav aria-label="Main navigation">
              <a href="/">Conversations</a>
              <a href="/agents">Agents</a>
              <a href="/search">Search</a>
            </nav>
            <div className="sidebar-foot">
              <p>All times in<br /><strong>America/New_York</strong></p>
              <a href="/auth/logout">Sign out ↗</a>
            </div>
          </aside>
          <main id="content">
            {children}
            <footer>BACKCHANNEL <span>Observe. Stay in the loop.</span></footer>
          </main>
        </div>
      </body>
    </html>
  );
}
