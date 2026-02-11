#!/bin/bash
# br-wrapper.sh - Wrapper for beads_rust (br) that adds commit reminder on close
#
# Install: alias br='/path/to/br-wrapper.sh' in your shell config
# Or: rename real br and put this in its place

REAL_BR="/Users/james/.local/bin/br"

# Pass all arguments to the real br
"$REAL_BR" "$@"
exit_code=$?

# If command was 'close' and succeeded, show reminder
if [[ "$1" == "close" && $exit_code -eq 0 ]]; then
    echo ""
    echo "📝 Remember to commit your changes if you haven't already"
fi

exit $exit_code
