"use client";

export default function ErrorPage() {
  return (
    <div className="error" role="alert">
      <h1>Unable to load this view</h1>
      <p>Check the page address or try again shortly. If the issue continues, the viewer’s connection may need attention.</p>
      <a className="button" href="/">Back to conversations</a>
    </div>
  );
}
