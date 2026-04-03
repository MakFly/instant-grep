class Ig < Formula
  desc "Trigram-indexed regex search CLI — sub-ms code search for AI agents"
  homepage "https://github.com/MakFly/instant-grep"
  version "1.6.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-aarch64"
      sha256 "9d2ff1534f58faa3c256d0d1fbf96b6ed1fb67ac78d8b5a572262ee4ebe6a9ac" # aarch64
    else
      url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-macos-x86_64"
      sha256 "0c23b632e3133e1d5ba6c461a076c3109888bd9b8eec9897ec8724b3cad1170c" # x86_64
    end
  end

  on_linux do
    url "https://github.com/MakFly/instant-grep/releases/download/v#{version}/ig-linux-x86_64"
    sha256 "9710fbc60be5aa18eaa7c6c99f108ba772d7d71849961cd8a41011e4b2e3402f" # linux
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
