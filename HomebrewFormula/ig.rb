class Ig < Formula
  desc "Trigram-indexed regex search CLI — sub-ms code search for AI agents"
  homepage "https://github.com/MakFly/instant-grep"
  version "1.4.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-aarch64"
      sha256 "7a7cd7e9d9089c324634bb2a2036dffa543a9da31e6f79554238fd91e1d39ac0" # aarch64
    else
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-x86_64"
      sha256 "9ee62d697a434e0a1a70ac8952c625e061a5009431009a0ad257d57d3cab593a" # x86_64
    end
  end

  on_linux do
    url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-linux-x86_64"
    sha256 "614deac576d1821722c27de433e62d7962b2b0fc9df2372f59ee1f6510671c82" # linux
  end

  def install
    bin.install Dir["*"].first => "ig"
  end

  def post_install
    system bin/"ig", "setup"
  end

  test do
    assert_match "ig #{version}", shell_output("#{bin}/ig --version")
  end
end
