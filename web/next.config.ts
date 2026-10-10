import type { NextConfig } from "next";
const config: NextConfig = {
    poweredByHeader: false,
    async headers() {
        return [{ source: "/:path*", headers: [
                    { key: "Cache-Control", value: "private, no-store, max-age=0" },
                    { key: "X-Content-Type-Options", value: "nosniff" },
                    { key: "X-Frame-Options", value: "DENY" },
                    { key: "Referrer-Policy", value: "no-referrer" },
                    { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
                ] }];
    },
};
export default config;
