const formatter = new Intl.DateTimeFormat("en-US", { timeZone: "America/New_York", month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit", timeZoneName: "short" });
export function timestamp(value: string): string { const date = new Date(value); return Number.isNaN(date.getTime()) ? "Unknown time" : formatter.format(date); }
