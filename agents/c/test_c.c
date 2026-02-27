#include "orionis.h"
#include <stdio.h>

void sub_func() {
    ORIONIS_TRACE();
    printf("Inside sub_func\n");
}

int main() {
    orionis_start("http://127.0.0.1:7700");
    
    ORIONIS_TRACE();
    printf("Inside main\n");
    sub_func();
    
    orionis_stop();
    return 0;
}
