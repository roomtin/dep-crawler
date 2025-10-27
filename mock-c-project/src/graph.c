#include "graph.h"
#include "util/math.h"

static int nodes = 0;
static int edges = 0;

void graph_init(void){
    nodes = 0;
    edges = 0;
}

int graph_add_node(int id){
    (void)id;
    nodes = add(nodes, 1);
    if (nodes > 1) edges = add(edges, 1);
    return nodes;
}

int graph_edge_count(void){
    return edges;
}
