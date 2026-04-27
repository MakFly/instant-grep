import { useState } from "react"
import { postSearch, type SearchResult } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Search, Loader2 } from "lucide-react"

const SAMPLE_QUERIES = [
  "compute hash of two consecutive bytes",
  "Unix socket daemon for IPC",
  "watch filesystem for changes and rebuild",
  "rank search results by relevance score",
]

export function SearchPage() {
  const [query, setQuery] = useState("")
  const [top, setTop] = useState(5)
  const [result, setResult] = useState<SearchResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (q?: string) => {
    const qq = q ?? query
    if (!qq.trim()) return
    setQuery(qq)
    setLoading(true)
    setError(null)
    try {
      setResult(await postSearch(qq, top))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Semantic search</h1>
        <p className="text-muted-foreground text-sm">
          Cosine top-N over the indexed chunks. Latency ≈ 200–800 ms (OpenAI embed +
          local cosine).
        </p>
      </div>

      <Card>
        <CardContent className="py-4">
          <form
            className="flex flex-col gap-3 sm:flex-row"
            onSubmit={(e) => {
              e.preventDefault()
              submit()
            }}
          >
            <Input
              placeholder="e.g. function that cancels a Stripe subscription"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="flex-1"
            />
            <Input
              type="number"
              min={1}
              max={20}
              value={top}
              onChange={(e) => setTop(Math.max(1, Math.min(20, Number(e.target.value) || 5)))}
              className="w-24"
            />
            <Button type="submit" disabled={loading || !query.trim()}>
              {loading ? <Loader2 className="size-4 animate-spin" /> : <Search className="size-4" />}
              Search
            </Button>
          </form>

          <div className="mt-3 flex flex-wrap gap-2">
            <span className="text-muted-foreground text-xs">Try:</span>
            {SAMPLE_QUERIES.map((s) => (
              <button
                key={s}
                onClick={() => submit(s)}
                className="text-muted-foreground hover:text-foreground hover:bg-accent rounded border px-2 py-0.5 text-xs transition"
              >
                {s}
              </button>
            ))}
          </div>
        </CardContent>
      </Card>

      {error && (
        <Card className="border-destructive/50">
          <CardContent className="text-destructive py-4 text-sm">{error}</CardContent>
        </Card>
      )}

      {result && (
        <>
          <div className="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-xs">
            <span>
              <strong className="text-foreground tabular-nums">{result.openai_ms}</strong> ms OpenAI
            </span>
            <span>
              <strong className="text-foreground tabular-nums">{result.cosine_ms}</strong> ms cosine
            </span>
            <span>
              <strong className="text-foreground tabular-nums">{result.scanned}</strong> chunks scanned
            </span>
            <span>
              <strong className="text-foreground tabular-nums">{result.query_tokens}</strong> tokens · $
              {result.query_cost_usd.toFixed(8)}
            </span>
          </div>

          <div className="space-y-3">
            {result.hits.map((h, i) => (
              <Card key={`${h.file}-${h.start_line}-${i}`}>
                <CardHeader className="pb-2">
                  <div className="flex items-center justify-between gap-2">
                    <CardTitle className="font-mono text-sm">
                      <span className="text-muted-foreground mr-2">#{i + 1}</span>
                      {h.file}
                      <span className="text-muted-foreground">
                        :{h.start_line}-{h.end_line}
                      </span>
                    </CardTitle>
                    <ScoreBadge score={h.score} />
                  </div>
                </CardHeader>
                <CardContent>
                  <pre className="bg-muted overflow-x-auto rounded p-3 text-xs leading-relaxed">
                    {h.preview.join("\n")}
                  </pre>
                </CardContent>
              </Card>
            ))}
            {result.hits.length === 0 && (
              <p className="text-muted-foreground text-sm">No hits.</p>
            )}
          </div>
        </>
      )}
    </div>
  )
}

function ScoreBadge({ score }: { score: number }) {
  const pct = Math.round(score * 100)
  const variant: "default" | "secondary" | "outline" =
    score >= 0.5 ? "default" : score >= 0.35 ? "secondary" : "outline"
  return (
    <Badge variant={variant} className="tabular-nums">
      {pct}% · {score.toFixed(4)}
    </Badge>
  )
}
