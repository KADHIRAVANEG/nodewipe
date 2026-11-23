class Nodewipe < Formula
  desc "Find and reclaim disk space from stray dev artifacts across 39+ ecosystems"
  homepage "https://github.com/KADHIRAVANEG/nodewipe"
  version "1.0.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-macos-aarch64"
      sha256 "1889b0863a531bc2553123c083721ac16dae08bd01acf7382a020b61efb97c1a"
    else
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-macos-x86_64"
      sha256 "6310f0c4cd4b0892c30d6378cfaeb4ac16baef0ecbd5cbcfd2539cc118088fed"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-linux-aarch64"
      sha256 "c5a314ea8f6edb12b7fff5f603a5c6b8fa13911f1d32aa46b9e19c91a40b8f64"
    else
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-linux-x86_64"
      sha256 "8b7830e08039409621e70ddb5ce6a72704519a6fcdff89b3b201c3c4fb4b41f9"
    end
  end

  def install
    bin_name = if OS.mac?
      Hardware::CPU.arm? ? "nodewipe-macos-aarch64" : "nodewipe-macos-x86_64"
    else
      Hardware::CPU.arm? ? "nodewipe-linux-aarch64" : "nodewipe-linux-x86_64"
    end

    bin.install bin_name => "nodewipe"

    # Install manual page
    (man1/"nodewipe.1").write Utils.safe_popen_read(bin/"nodewipe", "man")

    # Install shell completions
    (bash_completion/"nodewipe").write Utils.safe_popen_read(bin/"nodewipe", "completions", "bash")
    (zsh_completion/"_nodewipe").write Utils.safe_popen_read(bin/"nodewipe", "completions", "zsh")
    (fish_completion/"nodewipe.fish").write Utils.safe_popen_read(bin/"nodewipe", "completions", "fish")
  end

  test do
    assert_match "nodewipe #{version}", shell_output("#{bin}/nodewipe --version")
    assert_match "Usage:", shell_output("#{bin}/nodewipe --help")
  end
end
