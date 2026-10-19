class Hss < Formula
  desc "Terminal SSH server manager — TUI, credential storage, Ansible inventory"
  homepage "https://github.com/artp1ay/hss"
  version "1.0.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/artp1ay/hss/releases/download/v#{version}/hss-v#{version}-macos-aarch64.tar.gz"
      # sha256 "REPLACE_WITH_SHA256"
    else
      url "https://github.com/artp1ay/hss/releases/download/v#{version}/hss-v#{version}-macos-x86_64.tar.gz"
      # sha256 "REPLACE_WITH_SHA256"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/artp1ay/hss/releases/download/v#{version}/hss-v#{version}-linux-aarch64.tar.gz"
      # sha256 "REPLACE_WITH_SHA256"
    else
      url "https://github.com/artp1ay/hss/releases/download/v#{version}/hss-v#{version}-linux-x86_64.tar.gz"
      # sha256 "REPLACE_WITH_SHA256"
    end
  end

  def install
    bin.install "hss"

    if File.directory?("completions")
      bash_completion.install "completions/hss.bash" => "hss" if File.exist?("completions/hss.bash")
      zsh_completion.install "completions/_hss" => "_hss" if File.exist?("completions/_hss")
      fish_completion.install "completions/hss.fish" if File.exist?("completions/hss.fish")
    end

    if File.directory?("man")
      man1.install "man/hss.1" if File.exist?("man/hss.1")
    end
  end

  test do
    assert_match "hss", shell_output("#{bin}/hss --help")
  end
end
