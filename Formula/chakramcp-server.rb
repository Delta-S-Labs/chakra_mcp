# Homebrew formula for chakramcp-server — runs a private ChakraMCP
# network on the user's machine. Pairs with postgresql@16 (installed
# automatically as a dependency); each is started independently with
# `brew services`.
#
# Rendered + committed to Formula/chakramcp-server.rb on every cli-v*
# release by .github/workflows/cli-release.yml.

class ChakramcpServer < Formula
  desc "Self-hosted ChakraMCP relay (app + relay services in one process)"
  homepage "https://chakramcp.com"
  version "0.1.5"
  license "MIT"

  depends_on "postgresql@16"

  on_macos do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.5/chakramcp-server-0.1.5-aarch64-apple-darwin.tar.gz"
      sha256 "398a816852365f99df1e6f7c360c78ec1e5e131bdde0faf28b31fba5196d2cbf"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.5/chakramcp-server-0.1.5-x86_64-apple-darwin.tar.gz"
      sha256 "b4f7e012767157710e2d21a646f33049a44a66017caaffae05f87ea0da243303"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.5/chakramcp-server-0.1.5-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "3952f43bf542f3075bd036d3962bf14503bf99b2d706c6d771f6d904c8942f5a"
    end
    on_intel do
      url "https://github.com/Delta-S-Labs/chakra_mcp/releases/download/cli-v0.1.5/chakramcp-server-0.1.5-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "acde71d9fc890e2cf3bcf895de3593948f2e51bad90a956e4dd85dbac2e344e6"
    end
  end

  def install
    bin.install "chakramcp-server"
  end

  service do
    run [opt_bin/"chakramcp-server", "start"]
    keep_alive true
    log_path   var/"log/chakramcp-server.log"
    error_log_path var/"log/chakramcp-server.log"
  end

  def caveats
    <<~EOS
      First-time bootstrap (one-time):

        brew services start postgresql@16
        createdb chakramcp
        chakramcp-server init                    # writes ~/.chakramcp/server.toml
        chakramcp-server migrate                 # applies SQL migrations

      Then start it:

        brew services start chakramcp-server     # backgrounds the supervisor
        # — or run in the foreground for logs:
        chakramcp-server start

      The app service answers on http://localhost:8080 and the relay
      on http://localhost:8090. Point the CLI at it with:

        chakramcp networks add private \
          --app-url http://localhost:8080 \
          --relay-url http://localhost:8090
        chakramcp login --network private

      Edit ~/.chakramcp/server.toml to change ports, the JWT secret,
      or admin_email. The web UI is optional and isn't bundled — clone
      https://github.com/Delta-S-Labs/chakra_mcp and run pnpm dev under frontend/
      if you want it.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/chakramcp-server --version")
  end
end
