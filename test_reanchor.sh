#!/bin/bash
set -e

# Create test file
echo -e "line1\nline2\nline3\nline4\nline5" > testfile.txt

# Add to tracking
own add testfile.txt

# Create .own file with known content
cat > .own/testfile.txt.own << 'OWN'
snapshot: test
1:reviewed:hash1:alice:2024-01-20T10:00:00Z
3:approved:hash2:bob:2024-01-20T10:05:00Z
5:urgent:hash3:charlie:2024-01-20T10:10:00Z
OWN

echo "=== Initial state ==="
echo "File:"
cat testfile.txt
echo ""
echo ".own:"
cat .own/testfile.txt.own
echo ""

# Test 1: Add line at top
echo "=== Test 1: Add line at top ==="
echo -e "new line\nline1\nline2\nline3\nline4\nline5" > testfile.txt
# Run reanchor (would happen on next own review)
own status 2>/dev/null || true
echo "File after adding line:"
cat testfile.txt
echo ""

# Test 2: Delete line 3
echo "=== Test 2: Delete line 3 (line3) ==="
echo -e "new line\nline1\nline2\nline4\nline5" > testfile.txt
echo "File after deleting line3:"
cat testfile.txt
echo ""

echo "=== Summary ==="
echo "Re-anchoring should have:"
echo "1. Moved line1 review from 1->2"
echo "2. Removed line3 review (content deleted)"
echo "3. Moved line5 review from 5->5 (or 4 if we count properly)"
