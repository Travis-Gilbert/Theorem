import { redirect } from "next/navigation";

/** The console opens on the Canvas: a returning user's persistent spatial home,
 *  restored to exactly the way they left it. New users are routed to onboarding
 *  from the keys/claim flow. */
export default function Home() {
  redirect("/canvas");
}
