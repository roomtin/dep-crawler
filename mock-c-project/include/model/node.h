#pragma once
/* Intentional cycle: node.h includes edge.h and edge.h includes node.h.
   Include guards prevent redefinition, but your graph should still show a cycle. */
#include "model/edge.h"

typedef struct Node {
    int id;
} Node;
