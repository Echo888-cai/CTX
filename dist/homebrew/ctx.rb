# frozen_string_literal: true

class Ctx < Formula
  desc "Virtual memory for AI context"
  homepage "https://github.com/Echo888-cai/CTX"
  version "0.1.1"
  license "MIT"
  sha256 :no_check

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Echo888-cai/CTX/releases/download/v#{version}/CTX-Apple-Arm-cli-v#{version}.tar.gz"
    else
      url "https://github.com/Echo888-cai/CTX/releases/download/v#{version}/CTX-Apple-Intel-cli-v#{version}.tar.gz"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Echo888-cai/CTX/releases/download/v#{version}/CTX-Linux-Arm-v#{version}.tar.gz"
    else
      url "https://github.com/Echo888-cai/CTX/releases/download/v#{version}/CTX-Linux-x64-v#{version}.tar.gz"
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
