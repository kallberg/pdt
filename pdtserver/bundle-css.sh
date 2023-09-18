#!/bin/bash

cat src/css/reset.css src/css/style.css | lightningcss -m -o static/style.min.css