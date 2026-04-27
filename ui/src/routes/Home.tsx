import { useEffect, useState } from "react"
import { Link } from "react-router-dom"
import { getStatus, type Status } from "@/lib/api"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Database, Search, Sparkles, RefreshCw } from "lucide-react"

export function Home() {
  const [status, setStatus] = useState<Status | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const refresh = async () => {
    setLoading(true)
    setError(null)
    try {
      setStatus(await getStatus())
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    refresh()
  }, [])

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Embeddings POC</h1>
          <p className="text-muted-foreground text-sm">
            Cosine search over an OpenAI-embedded corpus. Pedagogical — see the JSON store,
            the latency, the cost.
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={refresh} disabled={loading}>
          <RefreshCw className={`size-4 ${loading ? "animate-spin" : ""}`} />
          Refresh
        </Button>
      </div>

      {error && (
        <Card className="border-destructive/50">
          <CardContent className="text-destructive py-4 text-sm">{error}</CardContent>
        </Card>
      )}

      {!loading && status && !status.ready && (
        <Card>
          <CardHeader>
            <CardTitle>No store yet</CardTitle>
            <CardDescription>{status.hint || "Run the indexer first."}</CardDescription>
          </CardHeader>
          <CardContent className="text-muted-foreground font-mono text-xs">
            <code className="bg-muted block rounded p-3">ig embed-poc index ./src</code>
          </CardContent>
        </Card>
      )}

      {!loading && status?.ready && (
        <>
          <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
            <StatCard label="Chunks" value={status.chunks?.toLocaleString()} icon={<Database className="size-4" />} />
            <StatCard label="Dim" value={status.dim?.toString()} />
            <StatCard label="Tokens" value={status.total_tokens?.toLocaleString()} />
            <StatCard
              label="Cost"
              value={`$${status.total_cost_usd?.toFixed(4)}`}
            />
          </div>

          <Card>
            <CardHeader>
              <CardTitle>Store</CardTitle>
              <CardDescription>Local JSON, human-readable.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-2 text-sm">
              <Row k="Provider" v={<Badge variant="secondary">{status.provider}</Badge>} />
              <Row k="Model" v={<code className="text-xs">{status.model}</code>} />
              <Row k="Version" v={<code className="text-xs">{status.version}</code>} />
              <Row
                k="Path"
                v={<code className="text-muted-foreground text-xs">{status.store_path}</code>}
              />
            </CardContent>
          </Card>

          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <Link to="/search">
              <Card className="hover:bg-accent/50 cursor-pointer transition">
                <CardHeader>
                  <CardTitle className="flex items-center gap-2">
                    <Search className="size-4" />
                    Semantic search
                  </CardTitle>
                  <CardDescription>
                    Ask in natural language, get top-N chunks ranked by cosine similarity.
                  </CardDescription>
                </CardHeader>
              </Card>
            </Link>
            <Link to="/inspect">
              <Card className="hover:bg-accent/50 cursor-pointer transition">
                <CardHeader>
                  <CardTitle className="flex items-center gap-2">
                    <Sparkles className="size-4" />
                    Inspect chunks
                  </CardTitle>
                  <CardDescription>
                    Browse the indexed chunks. Click one to see its embedding heatmap.
                  </CardDescription>
                </CardHeader>
              </Card>
            </Link>
          </div>
        </>
      )}
    </div>
  )
}

function StatCard({ label, value, icon }: { label: string; value?: string; icon?: React.ReactNode }) {
  return (
    <Card>
      <CardContent className="py-4">
        <div className="text-muted-foreground flex items-center gap-2 text-xs uppercase tracking-wide">
          {icon}
          {label}
        </div>
        <div className="mt-1 text-2xl font-semibold tabular-nums">{value ?? "—"}</div>
      </CardContent>
    </Card>
  )
}

function Row({ k, v }: { k: string; v: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4">
      <span className="text-muted-foreground">{k}</span>
      <span className="truncate text-right">{v}</span>
    </div>
  )
}
