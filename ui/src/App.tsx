import { BrowserRouter, Link, NavLink, Route, Routes } from "react-router-dom"
import { Home } from "@/routes/Home"
import { SearchPage } from "@/routes/SearchPage"
import { Inspect } from "@/routes/Inspect"
import { Database, Search, Sparkles } from "lucide-react"

export function App() {
  return (
    <BrowserRouter>
      <div className="min-h-svh">
        <header className="bg-background/80 sticky top-0 z-10 border-b backdrop-blur">
          <div className="mx-auto flex max-w-5xl items-center justify-between gap-6 px-6 py-3">
            <Link to="/" className="flex items-center gap-2 font-semibold">
              <Sparkles className="size-4 text-blue-500" />
              ig embed-poc
            </Link>
            <nav className="flex items-center gap-1 text-sm">
              <NavItem to="/" label="Home" icon={<Database className="size-4" />} />
              <NavItem to="/search" label="Search" icon={<Search className="size-4" />} />
              <NavItem to="/inspect" label="Inspect" icon={<Sparkles className="size-4" />} />
            </nav>
          </div>
        </header>
        <main className="mx-auto max-w-5xl px-6 py-8">
          <Routes>
            <Route path="/" element={<Home />} />
            <Route path="/search" element={<SearchPage />} />
            <Route path="/inspect" element={<Inspect />} />
          </Routes>
        </main>
      </div>
    </BrowserRouter>
  )
}

function NavItem({ to, label, icon }: { to: string; label: string; icon: React.ReactNode }) {
  return (
    <NavLink
      to={to}
      end={to === "/"}
      className={({ isActive }) =>
        `flex items-center gap-1.5 rounded-md px-3 py-1.5 transition ${
          isActive
            ? "bg-accent text-accent-foreground"
            : "text-muted-foreground hover:text-foreground hover:bg-accent/50"
        }`
      }
    >
      {icon}
      {label}
    </NavLink>
  )
}

export default App
