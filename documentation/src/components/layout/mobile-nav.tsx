import { useState, useEffect } from "react"

const navItems = [
  {
    section: "Overview",
    links: [{ href: "/docs/getting-started", label: "Getting Started" }],
  },
  {
    section: "Reference",
    links: [
      { href: "/docs/cli", label: "CLI Reference" },
      { href: "/docs/architecture", label: "Architecture" },
    ],
  },
  {
    section: "Advanced",
    links: [
      { href: "/docs/performance", label: "Performance" },
      { href: "/docs/agents", label: "Agent Integration" },
    ],
  },
]

export default function MobileNav() {
  const [open, setOpen] = useState(false)

  // Close on escape key
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false)
    }
    document.addEventListener("keydown", onKey)
    return () => document.removeEventListener("keydown", onKey)
  }, [])

  // Prevent body scroll when open
  useEffect(() => {
    document.body.style.overflow = open ? "hidden" : ""
    return () => { document.body.style.overflow = "" }
  }, [open])

  const currentPath = typeof window !== "undefined" ? window.location.pathname : ""

  return (
    <>
      {/* Hamburger button — only visible on mobile (lg:hidden) */}
      <button
        onClick={() => setOpen(!open)}
        className="lg:hidden flex items-center justify-center w-9 h-9 rounded-lg border border-border/50 hover:bg-secondary/50 transition-colors"
        aria-label="Toggle navigation"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
          {open ? (
            <>
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </>
          ) : (
            <>
              <line x1="3" y1="6" x2="21" y2="6" />
              <line x1="3" y1="12" x2="21" y2="12" />
              <line x1="3" y1="18" x2="21" y2="18" />
            </>
          )}
        </svg>
      </button>

      {/* Overlay */}
      {open && (
        <div
          className="fixed inset-0 z-40 bg-black/50 backdrop-blur-sm lg:hidden"
          onClick={() => setOpen(false)}
        />
      )}

      {/* Slide-in panel */}
      <div
        className={`fixed top-16 left-0 z-50 h-[calc(100vh-64px)] w-72 border-r border-border bg-background transform transition-transform duration-200 ease-in-out lg:hidden ${
          open ? "translate-x-0" : "-translate-x-full"
        }`}
      >
        <nav className="p-4 space-y-6 overflow-y-auto h-full">
          <div className="px-3">
            <a
              href="/"
              className="flex items-center gap-2 text-xs text-muted-foreground/60 hover:text-muted-foreground transition-colors font-mono"
              onClick={() => setOpen(false)}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M19 12H5M12 19l-7-7 7-7" />
              </svg>
              Back to home
            </a>
          </div>

          {navItems.map((section) => (
            <div key={section.section}>
              <p className="px-3 mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground/50 font-mono">
                {section.section}
              </p>
              <ul className="space-y-0.5">
                {section.links.map((link) => {
                  const active = currentPath === link.href || currentPath.startsWith(link.href + "/")
                  return (
                    <li key={link.href}>
                      <a
                        href={link.href}
                        onClick={() => setOpen(false)}
                        className={`flex items-center gap-2 px-3 py-2.5 rounded-lg text-sm transition-all ${
                          active
                            ? "sidebar-active font-medium"
                            : "text-muted-foreground hover:text-foreground hover:bg-secondary/50"
                        }`}
                      >
                        {active && (
                          <span className="w-1 h-1 rounded-full bg-primary flex-shrink-0" />
                        )}
                        {link.label}
                      </a>
                    </li>
                  )
                })}
              </ul>
            </div>
          ))}
        </nav>
      </div>
    </>
  )
}
