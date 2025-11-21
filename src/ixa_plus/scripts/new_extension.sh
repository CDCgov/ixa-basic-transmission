#!/bin/bash

# Script to generate a new Context extension
# Usage: mise run new:extension <name>

if [ $# -eq 0 ]; then
    echo "Error: Extension name is required"
    echo "Usage: mise run new:extension <name>"
    exit 1
fi

NAME="$1"
TEMPLATE_FILE="src/ixa_plus/templates/extension.rs"
OUTPUT_FILE="src/${NAME}.rs"

# Check if template exists
if [ ! -f "$TEMPLATE_FILE" ]; then
    echo "Error: Template file not found: $TEMPLATE_FILE"
    exit 1
fi

# Check if output file already exists
if [ -f "$OUTPUT_FILE" ]; then
    echo "Error: File already exists: $OUTPUT_FILE"
    exit 1
fi

# Convert name to camelCase (first letter lowercase)
# If first character is uppercase, make it lowercase
FIRST_CHAR=$(echo "$NAME" | cut -c1 | tr '[:upper:]' '[:lower:]')
REST=$(echo "$NAME" | cut -c2-)
CAMEL_NAME="${FIRST_CHAR}${REST}"

# Read template and replace EXT_NAME with camelCase name
sed "s/EXT_NAME/$CAMEL_NAME/g" "$TEMPLATE_FILE" > "$OUTPUT_FILE"

echo "Created extension: $OUTPUT_FILE"
echo "Trait name: ${CAMEL_NAME}Ext"
echo "RNG name: ${CAMEL_NAME}Rng"

