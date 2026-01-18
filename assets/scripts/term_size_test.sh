#!/bin/bash
# Terminal Size Test Script
# Run this in a workbench terminal to verify PTY/pane sizing

# Get terminal size
get_size() {
    local rows cols
    rows=$(tput lines 2>/dev/null || echo "?")
    cols=$(tput cols 2>/dev/null || echo "?")
    echo "$rows $cols"
}

# Draw a border to visualize the terminal area
draw_border() {
    local rows cols
    read -r rows cols < <(get_size)

    # Clear screen and hide cursor
    tput clear
    tput civis

    # Draw top border
    tput cup 0 0
    printf "+"
    for ((i=1; i<cols-1; i++)); do printf "-"; done
    printf "+"

    # Draw side borders
    for ((r=1; r<rows-1; r++)); do
        tput cup "$r" 0
        printf "|"
        tput cup "$r" $((cols-1))
        printf "|"
    done

    # Draw bottom border
    tput cup $((rows-1)) 0
    printf "+"
    for ((i=1; i<cols-1; i++)); do printf "-"; done
    printf "+"

    # Draw info in center
    local center_row=$((rows / 2))
    local info="Terminal: ${cols}x${rows}"
    local info_len=${#info}
    local start_col=$(( (cols - info_len) / 2 ))

    tput cup "$center_row" "$start_col"
    printf "\033[1;32m%s\033[0m" "$info"

    tput cup $((center_row + 1)) "$start_col"
    printf "stty: $(stty size 2>/dev/null || echo 'N/A')"

    tput cup $((center_row + 2)) "$start_col"
    printf "LINES=$LINES COLUMNS=$COLUMNS"

    tput cup $((center_row + 4)) "$start_col"
    printf "Press 'q' to quit, any key to refresh"

    # Mark corners with coordinates
    tput cup 1 2
    printf "(1,1)"

    tput cup 1 $((cols - 12))
    printf "(1,%d)" "$cols"

    tput cup $((rows - 2)) 2
    printf "(%d,1)" "$rows"

    tput cup $((rows - 2)) $((cols - 15))
    printf "(%d,%d)" "$rows" "$cols"

    # Show cursor at bottom for input
    tput cnorm
    tput cup $((rows - 1)) 2
}

# Cleanup on exit
cleanup() {
    tput cnorm  # Show cursor
    tput clear
    exit 0
}

trap cleanup INT TERM EXIT

# Main loop
while true; do
    draw_border
    read -rsn1 key
    if [[ "$key" == "q" || "$key" == "Q" ]]; then
        break
    fi
done
