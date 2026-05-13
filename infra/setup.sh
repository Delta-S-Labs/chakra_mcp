#!/usr/bin/env bash
# Bootstrap a fresh Lightsail (or any Debian/Ubuntu) box for the
# chakramcp-server stack. Run as root or with sudo.
#
# What it does:
#   1. Installs Docker + docker-compose plugin
#   2. Adds the default user to the docker group
#   3. Opens the firewall on 80/443 (idempotent)
#   4. Creates /opt/chakramcp and chowns to ec2-user (or ubuntu)
#
# It does NOT bring up the stack — that's a follow-up step after you
# scp the compose file + Caddyfile + .env.prod onto the box and pull
# the docker image from your registry.
#
# Usage on the box:
#   curl -fsSL https://raw.githubusercontent.com/Delta-S-Labs/chakra_mcp/main/infra/setup.sh | sudo bash
# or after scp'ing:
#   sudo bash setup.sh

set -euo pipefail

if [[ $EUID -ne 0 ]]; then
	echo "must run as root (sudo bash setup.sh)" >&2
	exit 1
fi

# ─── Detect the unprivileged user ──────────────────────────────
TARGET_USER=""
for candidate in ec2-user ubuntu admin debian; do
	if id "$candidate" &>/dev/null; then
		TARGET_USER="$candidate"
		break
	fi
done
if [[ -z "$TARGET_USER" ]]; then
	echo "couldn't find an unprivileged user (ec2-user/ubuntu/admin)" >&2
	exit 1
fi
echo "==> bootstrapping for user: $TARGET_USER"

# ─── Docker ───────────────────────────────────────────────────
if ! command -v docker &>/dev/null; then
	echo "==> installing docker"
	curl -fsSL https://get.docker.com | sh
fi

if ! docker compose version &>/dev/null; then
	echo "==> installing docker-compose plugin"
	apt-get update -y
	apt-get install -y docker-compose-plugin
fi

systemctl enable --now docker
usermod -aG docker "$TARGET_USER"

# ─── Firewall (UFW if installed; Lightsail manages its own too) ──
if command -v ufw &>/dev/null && ufw status | grep -q "Status: active"; then
	echo "==> opening 80/443 in ufw"
	ufw allow 80/tcp
	ufw allow 443/tcp
	ufw allow 443/udp  # HTTP/3
fi

# ─── Workdir ──────────────────────────────────────────────────
install -d -o "$TARGET_USER" -g "$TARGET_USER" -m 0755 /opt/chakramcp
echo "==> /opt/chakramcp ready (owned by $TARGET_USER)"

# ─── AWS CLI (optional, for ECR auth) ──────────────────────────
if ! command -v aws &>/dev/null; then
	echo "==> installing aws-cli for ECR auth"
	apt-get install -y unzip curl
	curl -fsSL "https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip" -o /tmp/aws.zip
	unzip -q /tmp/aws.zip -d /tmp
	/tmp/aws/install
	rm -rf /tmp/aws /tmp/aws.zip
fi

cat <<EOF

==> bootstrap complete

Next steps (run as $TARGET_USER):
  cd /opt/chakramcp

  # Copy these files in via scp from your laptop:
  #   docker-compose.prod.yml  → docker-compose.yml
  #   Caddyfile
  #   .env.prod                → .env

  # Authenticate against ECR (use the IAM role attached to this box,
  # or a key pair stored in ~/.aws/credentials):
  aws ecr get-login-password --region us-east-1 \\
    | docker login --username AWS --password-stdin \\
        877326604850.dkr.ecr.us-east-1.amazonaws.com

  # First-run migration:
  docker compose --profile migrate run --rm migrate

  # Bring it all up:
  docker compose up -d

  # Tail logs while DNS propagates and Caddy issues certs:
  docker compose logs -f caddy

EOF
