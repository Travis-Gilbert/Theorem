// Browser-side Gemma bridge (Phase 2). Any OpenAI-compatible chat endpoint
// serving Gemma 12B works: agentd's model loop, Ollama, or a hosted endpoint.
// Base-URL-swappable so agentd's local-Gemma state is never a hard blocker.
const BASE = ((import.meta.env.VITE_GEMMA_BASE_URL as string) || "http://localhost:11434/v1").replace(/\/$/, "");
const MODEL = (import.meta.env.VITE_GEMMA_MODEL as string) || "gemma2:9b";
const KEY = (import.meta.env.VITE_GEMMA_API_KEY as string) || "";

export async function askGemma(documentText: string, instruction: string): Promise<string> {
  const res = await fetch(`${BASE}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(KEY ? { Authorization: `Bearer ${KEY}` } : {}),
    },
    body: JSON.stringify({
      model: MODEL,
      temperature: 0.7,
      stream: false,
      messages: [
        {
          role: "system",
          content:
            "You are Gemma, a co-writer in a shared live document. Reply with ONLY the text to insert — no preamble, no surrounding quotes, no markdown fences.",
        },
        {
          role: "user",
          content: `Document so far:\n\n${documentText || "(empty)"}\n\nTask: ${instruction}`,
        },
      ],
    }),
  });
  if (!res.ok) {
    throw new Error(`endpoint ${res.status} ${res.statusText} — is ${BASE} serving an OpenAI-compatible Gemma?`);
  }
  const data = await res.json();
  const content = data?.choices?.[0]?.message?.content;
  if (typeof content !== "string") throw new Error("no completion in the response");
  return content.trim();
}
