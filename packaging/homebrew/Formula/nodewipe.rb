class Nodewipe < Formula
  desc "Find and reclaim disk space from stray dev artifacts across 39+ ecosystems"
  homepage "https://github.com/KADHIRAVANEG/nodewipe"
  version "0.9.9"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-macos-aarch64"
      sha256 "428a70d109c449c1a9e3c4b6f4499f2039846ac9da8786a03a79cbf8f9c287e7"
    else
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-macos-x86_64"
      sha256 "98569f66749a2de6afcb1983ece5a2bc9c3d5b4e36a44fc75ee17d6a639e6bab"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-linux-aarch64"
      sha256 "bb9ee6319e8adea6f6f8a786c093290f7901764ab2a5c88bce208cae25a3d37d"
    else
      url "https://github.com/KADHIRAVANEG/nodewipe/releases/download/v#{version}/nodewipe-linux-x86_64"
      sha256 "22e7e97ed58d031b67e5e07985082d58356173849cd61bc06a4ff1222dc8d29a"
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
