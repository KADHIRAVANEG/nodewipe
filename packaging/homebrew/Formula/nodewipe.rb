class Nodewipe < Formula
  desc "Find and reclaim disk space from stray dev artifacts across 13+ ecosystems"
  homepage "https://github.com/KADHIRAVANEG/nodewipe"
  version "0.9.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-macos-aarch64"
    else
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-macos-x86_64"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-linux-aarch64"
    else
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-linux-x86_64"
      sha256 "f2d14ffbddf59ce2a7cacb5ef021bf7b944300c1a0b3a8aef9ce932c396afe63"
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
