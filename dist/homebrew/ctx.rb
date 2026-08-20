# frozen_string_literal: true

class Ctx < Formula
  desc "Virtual memory for AI context"
  homepage "https://github.com/Echo888-cai/CTX"
  version "0.2.1"
  license "MIT"
  sha256 :no_check

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Echo888-cai/CTX/releases/download/v#{version}/ctx-aarch64-apple-darwin.tar.gz"
    else
      url "https://github.com/Echo888-cai/CTX/releases/download/v#{version}/ctx-x86_64-apple-darwin.tar.gz"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Echo888-cai/CTX/releases/download/v#{version}/ctx-aarch64-unknown-linux-gnu.tar.gz"
    else
      url "https://github.com/Echo888-cai/CTX/releases/download/v#{version}/ctx-x86_64-unknown-linux-gnu.tar.gz"
    end
  end

  def install
    binary = Dir["ctx*"].find { |f| File.file?(f) && File.executable?(f) } || "ctx"
    bin.install binary => "ctx"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/ctx --version")
  end
end
