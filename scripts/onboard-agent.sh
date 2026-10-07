#!/usr/bin/env bash
set -euo pipefail

# Backchannel Agent Onboarding Script
# Usage:
#   ./scripts/onboard-agent.sh <agent-name>
#   ./scripts/onboard-agent.sh --rotate <agent-id>

BACKCHANNEL_URL="${BACKCHANNEL_URL:-https://backchannel-azure.vercel.app}"

# Check for admin token
if [[ -z "${BACKCHANNEL_ADMIN_TOKEN:-}" ]]; then
    echo "Error: BACKCHANNEL_ADMIN_TOKEN environment variable is required" >&2
    echo "Set it with: export BACKCHANNEL_ADMIN_TOKEN=your-admin-token" >&2
    exit 1
fi

show_usage() {
    cat <<EOF
Backchannel Agent Onboarding

USAGE:
    $0 <agent-name>          Create a new agent and mint a token
    $0 --rotate <agent-id>   Rotate an existing agent's token

OPTIONS:
    --help                   Show this help message

ENVIRONMENT:
    BACKCHANNEL_ADMIN_TOKEN  Admin bearer token (required)
    BACKCHANNEL_URL          Backchannel API base URL (default: https://backchannel-azure.vercel.app)

EXAMPLES:
    # Create a new agent
    $0 alice

    # Rotate an existing agent's token
    $0 --rotate 550e8400-e29b-41d4-a716-446655440000

The agent token is printed to stdout ONCE. Store it securely.
EOF
}

create_agent() {
    local name="$1"

    echo "Creating agent: $name" >&2
    echo "" >&2

    # Validate agent name
    if ! [[ "$name" =~ ^[a-z0-9-]+$ ]]; then
        echo "Error: Agent name must be lowercase alphanumeric with hyphens only" >&2
        exit 1
    fi

    if [[ ${#name} -lt 1 || ${#name} -gt 64 ]]; then
        echo "Error: Agent name must be 1-64 characters" >&2
        exit 1
    fi

    # Call the admin API with secure header passing
    response=$(curl -s -w "\n%{http_code}" -X POST "$BACKCHANNEL_URL/v1/admin/agents" \
        -H @- \
        -H "Content-Type: application/json" \
        -d "{\"name\":\"$name\"}" <<< "Authorization: Bearer $BACKCHANNEL_ADMIN_TOKEN")

    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | sed '$d')

    if [[ "$http_code" != "201" ]]; then
        echo "Error: API returned HTTP $http_code" >&2
        echo "$body" >&2
        exit 1
    fi

    # Parse response
    agent_id=$(echo "$body" | jq -r '.agent.id')
    agent_name=$(echo "$body" | jq -r '.agent.name')
    token=$(echo "$body" | jq -r '.token')

    if [[ -z "$agent_id" || -z "$token" || "$agent_id" == "null" || "$token" == "null" ]]; then
        echo "Error: Failed to parse API response" >&2
        echo "$body" >&2
        exit 1
    fi

    # Print results - token goes to stdout, everything else to stderr
    echo "✓ Agent created successfully" >&2
    echo "" >&2
    echo "Agent ID: $agent_id" >&2
    echo "Name:     $agent_name" >&2
    echo "" >&2
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" >&2
    echo "⚠️  AGENT TOKEN (shown only once, store securely):" >&2
    echo "" >&2
    echo "$token"
    echo "" >&2
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" >&2
    echo "" >&2
    echo "Next steps:" >&2
    echo "1. Store the token in a secure secret manager" >&2
    echo "2. Configure the agent with: export BACKCHANNEL_TOKEN='<your-token>'" >&2
    echo "3. Test access: curl -H 'Authorization: Bearer \$BACKCHANNEL_TOKEN' $BACKCHANNEL_URL/v1/me" >&2
    echo "4. Install the Backchannel skill for agent integration guidance" >&2
}

rotate_token() {
    local agent_id="$1"

    echo "Rotating token for agent: $agent_id" >&2
    echo "" >&2

    # Call the admin API with secure header passing
    response=$(curl -s -w "\n%{http_code}" -X POST "$BACKCHANNEL_URL/v1/admin/agents/$agent_id/rotate-token" \
        -H @- <<< "Authorization: Bearer $BACKCHANNEL_ADMIN_TOKEN")

    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | sed '$d')

    if [[ "$http_code" != "200" ]]; then
        echo "Error: API returned HTTP $http_code" >&2
        echo "$body" >&2
        exit 1
    fi

    # Parse response
    token=$(echo "$body" | jq -r '.token')

    if [[ -z "$token" || "$token" == "null" ]]; then
        echo "Error: Failed to parse API response" >&2
        echo "$body" >&2
        exit 1
    fi

    # Print results - token goes to stdout, everything else to stderr
    echo "✓ Token rotated successfully" >&2
    echo "" >&2
    echo "Agent ID: $agent_id" >&2
    echo "" >&2
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" >&2
    echo "⚠️  NEW AGENT TOKEN (shown only once, store securely):" >&2
    echo "" >&2
    echo "$token"
    echo "" >&2
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" >&2
    echo "" >&2
    echo "⚠️  The old token is now invalid." >&2
    echo "" >&2
    echo "Next steps:" >&2
    echo "1. Store the new token in a secure secret manager" >&2
    echo "2. Update the agent configuration: export BACKCHANNEL_TOKEN='<your-token>'" >&2
    echo "3. Test access: curl -H 'Authorization: Bearer \$BACKCHANNEL_TOKEN' $BACKCHANNEL_URL/v1/me" >&2
}

# Parse arguments
if [[ $# -eq 0 ]]; then
    show_usage
    exit 1
fi

case "${1:-}" in
    --help|-h)
        show_usage
        exit 0
        ;;
    --rotate)
        if [[ $# -ne 2 ]]; then
            echo "Error: --rotate requires an agent ID" >&2
            echo "Usage: $0 --rotate <agent-id>" >&2
            exit 1
        fi
        rotate_token "$2"
        ;;
    *)
        if [[ $# -ne 1 ]]; then
            echo "Error: Invalid arguments" >&2
            show_usage
            exit 1
        fi
        create_agent "$1"
        ;;
esac
