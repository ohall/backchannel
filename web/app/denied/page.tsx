import { Heading } from "@/components/view";

export default function Denied() {
  return (
    <>
      <Heading eyebrow="Private workspace" title="Access isn’t available">
        <p>Sign in with the approved account and a verified email address. Your session may also have expired or sign-in may have been canceled.</p>
      </Heading>
      <a className="button" href="/auth/logout">Sign out and try again</a>
    </>
  );
}
