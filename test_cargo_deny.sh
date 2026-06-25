#!/bin/bash
# Test script for cargo-deny configuration

echo "=== Testing Cargo Deny Configuration ==="
echo ""

# Check if cargo-deny is installed
if ! command -v cargo-deny &> /dev/null; then
    echo "cargo-deny not found. Installing..."
    cargo install cargo-deny
fi

echo "Running cargo deny check..."
echo ""

# Run cargo deny check
cargo deny check

if [ $? -eq 0 ]; then
    echo ""
    echo "✅ cargo deny check PASSED"
else
    echo ""
    echo "❌ cargo deny check FAILED"
    echo ""
    echo "Common fixes:"
    echo "1. Update deny.toml to allow additional licenses if needed"
    echo "2. Add duplicate versions to skip list"
    echo "3. Check for yanked dependencies in Cargo.lock"
fi