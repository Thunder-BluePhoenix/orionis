#include "../../agents/cpp/orionis.hpp"
#include <iostream>
#include <stdexcept>

void buggy_function() {
    ORION_TRACE();
    std::cout << "Handling C++ request...\n";
    // Deliberate untrapped exception
    throw std::runtime_error("Something went terribly wrong!");
}

void entry_point() {
    ORION_TRACE();
    buggy_function();
}

int main() {
    orionis::start();
    std::cout << "Starting C++ process... Will throw exception shortly!\n";
    
    // Give the background thread time to hook up
    std::this_thread::sleep_for(std::chrono::milliseconds(500));
    
    try {
        entry_point();
    } catch (const std::exception& e) {
        orionis::Event ev;
        ev.trace_id    = orionis::trace_id;
        ev.span_id     = orionis::new_uuid();
        ev.timestamp_ms= orionis::now_ms();
        ev.event_type  = "exception";
        ev.function_name = "main_catch";
        ev.file        = "main";
        ev.line        = 0;
        ev.error_message = e.what();
        ev.duration_us = -1;
        orionis::enqueue(ev);
        orionis::flush();
        std::cerr << "[Orionis] Caught exception: " << e.what() << "\n";
    }
    
    orionis::stop();
    return 0;
}
