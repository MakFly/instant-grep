class Ig < Formula
  desc "Trigram-indexed regex search CLI — sub-ms code search for AI agents"
  homepage "https://github.com/MakFly/instant-grep"
  version "1.3.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-aarch64"
      sha256 "2c50108a8d56c24dc21aad015a0f4ac44102a0cd9a15aadcc8d30916dcbaf11b" # aarch64
    else
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-x86_64"
      sha256 "53cf6b2aefa4b6fe81e7d311b53df83d14ed19644d25d8d5eea61a3de26e97f1" # x86_64
    end
  end

  on_linux do
    url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-linux-x86_64"
    sha256 "9a975464f0bb4d516e64a966e878e05963d0cd090482b7c99342e1558dd1df01" # linux
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
