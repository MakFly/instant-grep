export type Status = {
  ready: boolean
  version?: string
  provider?: string
  model?: string
  dim?: number
  chunks?: number
  total_tokens?: number
  total_cost_usd?: number
  store_path?: string
  hint?: string
  error?: string
}

export type Hit = {
  score: number
  file: string
  start_line: number
  end_line: number
  tokens: number
  preview: string[]
}

export type SearchResult = {
  query: string
  query_tokens: number
  query_cost_usd: number
  openai_ms: number
  cosine_ms: number
  scanned: number
  hits: Hit[]
}

export type Chunk = {
  id: number
  file: string
  start_line: number
  end_line: number
  tokens: number
  embedding: number[]
}

export type ChunksResult = {
  total: number
  returned: number
  dim: number
  chunks: Chunk[]
}

export async function getStatus(): Promise<Status> {
  const r = await fetch("/api/status")
  if (!r.ok) throw new Error(`status ${r.status}`)
  return r.json()
}

export async function postSearch(query: string, top: number): Promise<SearchResult> {
  const r = await fetch("/api/search", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query, top }),
  })
  if (!r.ok) {
    const err = await r.json().catch(() => ({ error: r.statusText }))
    throw new Error(err.error || `HTTP ${r.status}`)
  }
  return r.json()
}

export async function getChunks(limit: number): Promise<ChunksResult> {
  const r = await fetch(`/api/chunks?limit=${limit}`)
  if (!r.ok) throw new Error(`chunks ${r.status}`)
  return r.json()
}
