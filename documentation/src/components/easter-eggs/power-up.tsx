"use client"

import { useState } from "react"

interface PowerUpProps {
  message?: string
  trigger?: string
}

export function PowerUpButton({ message = "KAIOKEN!", trigger = "Click" }: PowerUpProps) {
  const [active, setActive] = useState(false)

  function activate() {
    setActive(true)
    setTimeout(() => setActive(false), 1500)
  }

  return (
    <button
      onClick={activate}
      style={{
        position: "relative",
        padding: "0.75rem 2rem",
        borderRadius: "0.75rem",
        fontFamily: "'JetBrains Mono Variable', monospace",
        fontWeight: 700,
        fontSize: "0.875rem",
        background: active
          ? "linear-gradient(135deg, #FFD700, #FF6B00)"
          : "linear-gradient(135deg, #FF6B00, #FF9500)",
        color: "#000",
        border: "none",
        cursor: "pointer",
        transition: "all 0.2s ease",
        boxShadow: active
          ? "0 0 40px rgba(255,215,0,0.6), 0 0 80px rgba(255,107,0,0.3)"
          : "0 0 20px rgba(255,107,0,0.4)",
        transform: active ? "scale(1.05)" : "scale(1)",
      }}
    >
      {active ? message : trigger}
    </button>
  )
}

export function ChakraLoader({ text = "Gathering chakra..." }: { text?: string }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "0.75rem",
        padding: "0.75rem 1.25rem",
        borderRadius: "0.5rem",
        background: "rgba(255,107,0,0.05)",
        border: "1px solid rgba(255,107,0,0.2)",
        fontFamily: "'JetBrains Mono Variable', monospace",
        fontSize: "0.875rem",
        color: "#FF9500",
      }}
    >
      <div
        style={{
          width: "16px",
          height: "16px",
          borderRadius: "50%",
          border: "2px solid rgba(255,107,0,0.3)",
          borderTop: "2px solid #FF6B00",
          animation: "spin 0.6s linear infinite",
        }}
      />
      {text}
    </div>
  )
}
