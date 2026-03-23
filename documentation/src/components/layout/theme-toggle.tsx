import { useEffect, useState } from "react"

export default function ThemeToggle() {
  const [isDark, setIsDark] = useState(true)

  useEffect(() => {
    // On mount: read persisted preference or system preference
    const stored = localStorage.getItem("theme")
    if (stored === "light") {
      setIsDark(false)
      document.documentElement.classList.remove("dark")
    } else if (stored === "dark") {
      setIsDark(true)
      document.documentElement.classList.add("dark")
    } else {
      // No stored preference: default dark, but respect system prefers-color-scheme
      const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches
      const useDark = prefersDark !== false // true unless explicitly light
      if (!prefersDark) {
        setIsDark(false)
        document.documentElement.classList.remove("dark")
      } else {
        setIsDark(true)
        document.documentElement.classList.add("dark")
      }
    }
  }, [])

  function toggle() {
    const next = !isDark
    setIsDark(next)
    if (next) {
      document.documentElement.classList.add("dark")
      localStorage.setItem("theme", "dark")
    } else {
      document.documentElement.classList.remove("dark")
      localStorage.setItem("theme", "light")
    }
  }

  return (
    <button
      onClick={toggle}
      aria-label={isDark ? "Switch to light mode" : "Switch to dark mode"}
      title={isDark ? "Switch to light mode" : "Switch to dark mode"}
      class="relative flex items-center justify-center w-9 h-9 rounded-lg border border-border/50 text-muted-foreground hover:text-foreground hover:border-[rgba(255,107,0,0.4)] hover:bg-[rgba(255,107,0,0.05)] transition-all duration-200 group"
    >
      {/* Sun icon — shown in dark mode (click to go light) */}
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        style={{
          position: "absolute",
          transition: "opacity 0.2s ease, transform 0.3s ease",
          opacity: isDark ? 1 : 0,
          transform: isDark ? "rotate(0deg) scale(1)" : "rotate(90deg) scale(0.5)",
        }}
      >
        <circle cx="12" cy="12" r="4" />
        <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
      </svg>

      {/* Moon icon — shown in light mode (click to go dark) */}
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        style={{
          position: "absolute",
          transition: "opacity 0.2s ease, transform 0.3s ease",
          opacity: isDark ? 0 : 1,
          transform: isDark ? "rotate(-90deg) scale(0.5)" : "rotate(0deg) scale(1)",
        }}
      >
        <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
      </svg>
    </button>
  )
}
