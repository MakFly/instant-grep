import { useEffect, useState } from "react"
import { getChunks, type Chunk, type ChunksResult } from "@/lib/api"
import { Card, CardContent } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Loader2 } from "lucide-react"

export function Inspect() {
  const [data, setData] = useState<ChunksResult | null>(null)
  const [limit, setLimit] = useState(100)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [open, setOpen] = useState<Chunk | null>(null)

  const load = async (n: number) => {
    setLoading(true)
    setError(null)
    try {
      setData(await getChunks(n))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    load(limit)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div className="space-y-6">
      <div className="flex items-end justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Inspect chunks</h1>
          <p className="text-muted-foreground text-sm">
            Click any chunk to render its 1536-D embedding as a 32×48 heatmap.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-muted-foreground text-xs">Limit</span>
          {[50, 100, 250, 500].map((n) => (
            <Button
              key={n}
              size="sm"
              variant={limit === n ? "default" : "outline"}
              onClick={() => {
                setLimit(n)
                load(n)
              }}
            >
              {n}
            </Button>
          ))}
        </div>
      </div>

      {error && (
        <Card className="border-destructive/50">
          <CardContent className="text-destructive py-4 text-sm">{error}</CardContent>
        </Card>
      )}

      {loading && (
        <div className="text-muted-foreground flex items-center gap-2 text-sm">
          <Loader2 className="size-4 animate-spin" /> loading…
        </div>
      )}

      {data && (
        <>
          <div className="text-muted-foreground text-xs">
            Showing <strong className="text-foreground">{data.returned}</strong> of{" "}
            <strong className="text-foreground">{data.total}</strong> · dim {data.dim}
          </div>

          <Card>
            <CardContent className="p-0">
              <div className="divide-y">
                {data.chunks.map((c) => (
                  <button
                    key={c.id}
                    onClick={() => setOpen(c)}
                    className="hover:bg-accent flex w-full items-center justify-between gap-4 px-4 py-2 text-left transition"
                  >
                    <div className="min-w-0 flex-1 truncate font-mono text-xs">
                      <span className="text-muted-foreground mr-2">#{c.id}</span>
                      {c.file}
                      <span className="text-muted-foreground">
                        :{c.start_line}-{c.end_line}
                      </span>
                    </div>
                    <Badge variant="outline" className="tabular-nums">
                      {c.tokens} tok
                    </Badge>
                  </button>
                ))}
              </div>
            </CardContent>
          </Card>
        </>
      )}

      <Dialog open={!!open} onOpenChange={(v) => !v && setOpen(null)}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle className="font-mono text-sm">
              {open && (
                <>
                  #{open.id} {open.file}
                  <span className="text-muted-foreground">
                    :{open.start_line}-{open.end_line}
                  </span>
                </>
              )}
            </DialogTitle>
            <DialogDescription>
              {open && (
                <>
                  {open.embedding.length} dimensions · {open.tokens} tokens · L2 ={" "}
                  {l2Norm(open.embedding).toFixed(4)}
                </>
              )}
            </DialogDescription>
          </DialogHeader>
          {open && <Heatmap values={open.embedding} />}
        </DialogContent>
      </Dialog>
    </div>
  )
}

function l2Norm(v: number[]): number {
  return Math.sqrt(v.reduce((a, x) => a + x * x, 0))
}

function Heatmap({ values }: { values: number[] }) {
  // 1536 = 32 cols × 48 rows
  const cols = 32
  const rows = Math.ceil(values.length / cols)
  // Symmetrical scale around 0; clamp to ±max for stable colors
  const max = Math.max(...values.map(Math.abs)) || 1

  return (
    <div className="space-y-2">
      <div
        className="grid gap-px bg-border p-px"
        style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
      >
        {Array.from({ length: rows * cols }, (_, i) => {
          const v = values[i] ?? 0
          const t = v / max // [-1, 1]
          const color = colorFor(t)
          return (
            <div
              key={i}
              className="aspect-square"
              style={{ background: color }}
              title={`d${i} = ${v.toFixed(4)}`}
            />
          )
        })}
      </div>
      <div className="text-muted-foreground flex items-center justify-between text-xs">
        <span>−{max.toFixed(3)}</span>
        <Legend />
        <span>+{max.toFixed(3)}</span>
      </div>
    </div>
  )
}

function colorFor(t: number): string {
  // Diverging blue → black → red
  const v = Math.max(-1, Math.min(1, t))
  if (v >= 0) {
    const a = v
    const r = Math.round(255 * a)
    const g = Math.round(40 * (1 - a))
    const b = Math.round(40 * (1 - a))
    return `rgb(${r},${g},${b})`
  } else {
    const a = -v
    const r = Math.round(40 * (1 - a))
    const g = Math.round(120 * (1 - a))
    const b = Math.round(255 * a + 60 * (1 - a))
    return `rgb(${r},${g},${b})`
  }
}

function Legend() {
  const stops = Array.from({ length: 32 }, (_, i) => -1 + (2 * i) / 31)
  return (
    <div className="flex h-2 flex-1 mx-3 overflow-hidden rounded">
      {stops.map((t, i) => (
        <div key={i} className="flex-1" style={{ background: colorFor(t) }} />
      ))}
    </div>
  )
}
