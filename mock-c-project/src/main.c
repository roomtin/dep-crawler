#include "common.h"
#include "graph.h"
#include <stdio.h>
#include "../plugins/plugin.h"

extern const char* current_platform(void);

const char* orchard_version(void){
    return ORCHARD_VERSION; /* from generated/autogen.h via macro include */
}
//
int main(void){
    graph_init();
    graph_add_node(1);
    graph_add_node(2);
    plugin_answer();
    printf("Orchard %s on %s: edges=%d\n",
           orchard_version(), current_platform(), graph_edge_count());
    return 0;
}
