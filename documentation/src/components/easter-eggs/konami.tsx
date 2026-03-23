"use client"

import { useEffect, useState, useCallback } from "react"

const KONAMI_SEQUENCE = [
  "ArrowUp", "ArrowUp",
  "ArrowDown", "ArrowDown",
  "ArrowLeft", "ArrowRight",
  "ArrowLeft", "ArrowRight",
  "b", "a",
]

export default function KonamiCode() {
  const [progress, setProgress] = useState(0)
  const [triggered, setTriggered] = useState(false)

  const handleKey = useCallback(
    (e: KeyboardEvent) => {
      const key = e.key
      if (key === KONAMI_SEQUENCE[progress]) {
        const nextProgress = progress + 1
        setProgress(nextProgress)
        if (nextProgress === KONAMI_SEQUENCE.length) {
          setTriggered(true)
          setProgress(0)
          setTimeout(() => setTriggered(false), 2500)
        }
      } else {
        setProgress(key === KONAMI_SEQUENCE[0] ? 1 : 0)
      }
    },
    [progress]
  )

  useEffect(() => {
    window.addEventListener("keydown", handleKey)
    return () => window.removeEventListener("keydown", handleKey)
  }, [handleKey])

  if (!triggered) return null

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 9999,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        pointerEvents: "none",
        animation: "power-flash 2.5s ease forwards",
        background: "radial-gradient(circle, rgba(255,215,0,0.15) 0%, rgba(255,107,0,0.1) 40%, transparent 70%)",
      }}
    >
      <div
        style={{
          fontFamily: "'JetBrains Mono Variable', monospace",
          fontSize: "clamp(2rem, 8vw, 5rem)",
          fontWeight: 900,
          color: "#FFD700",
          textShadow: "0 0 40px #FF6B00, 0 0 80px #FF6B00, 0 0 120px rgba(255,107,0,0.5)",
          animation: "kaioken-text 2.5s ease forwards",
          letterSpacing: "0.05em",
          textAlign: "center",
          lineHeight: 1.2,
        }}
      >
        KAIOKEN x20!
      </div>
      <div
        style={{
          marginTop: "1rem",
          fontFamily: "'JetBrains Mono Variable', monospace",
          fontSize: "1rem",
          color: "#FF9500",
          animation: "kaioken-text 2.5s ease 0.2s forwards",
          opacity: 0,
          textShadow: "0 0 20px rgba(255,107,0,0.8)",
        }}
      >
        Power Level: OVER 9000!!!
      </div>

      {/* Energy rings */}
      {[1, 2, 3].map((i) => (
        <div
          key={i}
          style={{
            position: "absolute",
            width: `${200 * i}px`,
            height: `${200 * i}px`,
            borderRadius: "50%",
            border: "2px solid rgba(255,215,0,0.3)",
            animation: `ki-pulse ${0.5 + i * 0.3}s ease-in-out infinite`,
            boxShadow: "0 0 20px rgba(255,215,0,0.2)",
          }}
        />
      ))}
    </div>
  )
}
