/** Onboarding (claim / first-run) sits outside the shell: no rail, no island,
 *  just the ambient field and a centered claim flow modeled on Browser Use. */
export default function OnboardingLayout({ children }: { children: React.ReactNode }) {
  return <div className="min-h-dvh w-full">{children}</div>;
}
