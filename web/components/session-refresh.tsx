"use client";
import { useEffect } from "react";
// Revalidate restored documents and discard visible protected content on logout or expiry.
export function SessionRefresh({ expiresAt }: {
    expiresAt?: number;
}) {
    useEffect(() => {
        const hide = () => { document.documentElement.style.visibility = "hidden"; };
        const reload = () => { hide(); window.location.reload(); };
        const show = (event: PageTransitionEvent) => { if (event.persisted)
            reload(); };
        const visibility = () => { if (document.visibilityState === "visible" && expiresAt)
            reload(); };
        window.addEventListener("focus", visibility);
        window.addEventListener("pagehide", hide);
        window.addEventListener("pageshow", show);
        document.addEventListener("visibilitychange", visibility);
        const timer = expiresAt ? window.setTimeout(reload, Math.max(0, expiresAt - Date.now())) : undefined;
        return () => { window.removeEventListener("focus", visibility); window.removeEventListener("pagehide", hide); window.removeEventListener("pageshow", show); document.removeEventListener("visibilitychange", visibility); if (timer)
            window.clearTimeout(timer); };
    }, [expiresAt]);
    return null;
}
