# SigmaOS-sigma-kernel Component Wiki

## Overview
The SigmaOS-sigma-kernel is the core kernel component of the SigmaOS sovereign operating system. This improved kernel implementation incorporates design principles from Linux, Arch Linux, and Omarchy Linux, providing a robust, performant, and modular foundation for the operating system. The kernel supports the SigmaOS component architecture and enables efficient resource management, process scheduling, memory handling, and hardware abstraction.

## Principles
- **Single Responsibility**: The kernel has a single well-defined purpose: providing core operating system services to all SigmaOS components, including process management, memory management, hardware abstraction, and system calls
- **Abstraction**: Clear abstraction over the component's APIs and functionality, hiding implementation details from upper-layer components and applications
- **Fail-Fast on Initialization Errors**: Fail-fast principle for initialization errors — the kernel panics immediately rather than continuing in an inconsistent state, following Linux kernel philosophy and Omarchy sovereign design
- **Deterministic Setup**: Deterministic initialization and configuration — the kernel boot process is reproducible and predictable, reflecting Arch Linux's KISS principle and SigmaOS's goal-based architecture
- **Clear Abstraction Over APIs**: Well-defined, clear system call interface and kernel APIs that are stable and versioned, enabling component compatibility across SigmaOS releases
- **Composition over Inheritance for Extensions**: Use composition patterns for kernel extensions and drivers rather than class inheritance, following SigmaOS modular architecture principles and Linux kernel module philosophy

## Architecture
SigmaOS-sigma-kernel follows a modern microkernel-inspired architecture with monolithic core elements, structured as follows:

### Core Subsystems
- **Process Scheduler**: Cooperative and preemptive scheduling inspired by Linux CFS (Completely Fair Scheduler), with priority-based scheduling and tickless timer support (Arch Linux performance-oriented design)
- **Memory Management**: Buddy allocator for kernel memory with page-level granularity, inspired by Linux buddy allocator and Arch Linux VM subsystem
- **Virtual Memory Paging**: Multi-level page table management (4-level on x86-64) with identity mapping and physical memory tracking, following Linux paging architecture
- **Interrupt Framework**: Comprehensive IRQ handling with x86 exception handlers, programmable interrupt controller (PIC) initialization, and kernel-level interrupt routing, drawing from Linux irq_subsystem
- **Timer Subsystem**: Programmable Interval Timer (PIT) configured for 1ms tick with support for high-resolution timers, inspired by Linux kernel timer subsystem
- **System Call Dispatcher**: ~200 system call numbers providing process, file, memory, network, and signal operations, with proper error handling and validation, mirroring Linux syscall table design
- **Framebuffer Support**: VGA text mode and basic framebuffer initialization for graphical output, with VESA compatibility

### Boot Process
1. Bootloader (GRUB2) loads kernel stub into memory at 1MB+ boundary
2. GDT (Global Descriptor Table) initialization with code/data segment descriptors
3. IDT (Interrupt Descriptor Table) setup with all x86 exception handlers
4. PIC (Programmable Interrupt Controller) cascading configuration
5. Timer (PIT) setup for kernel tick generation
6. Physical memory range detection and page allocation
7. Paging initialization with multi-level page tables
8. Kernel enters main loop with scheduler and event processing

### Hardware Abstraction
- **x86 Specific**: GDT, IDT, PIC, PIT, MSR access, CPUID detection
- **Generic**: Hardware info structure, CPU identification, feature flags
- **Device Model**: Future extension point for device driver model

### Kernel Services Exported to Components
- System call interface for process creation, termination, synchronization
- Memory allocation APIs (kernel heap management)
- Interrupt request handlers
- Timer services
- Hardware information queries

## API
The kernel provides the following public system calls and interfaces:

### Process Management
- `SIGMA_SYS_GETPID` - Get process ID
- `SIGMA_SYS_EXIT` - Exit process with status code
- `SIGMA_SYS_GETPPID` - Get parent process ID
- `SIGMA_SYS_SLEEP` - Sleep for specified ticks
- `SIGMA_SYS_WAKEUP` - Wake up a process

### Memory Management
- `SIGMA_SYS_BRK` - Program break (heap management)
- `SIGMA_SYS_SBRK` - Set program break (extend/shrink heap)
- `SIGMA_SYS_MMAP` - Memory mapping (stub implementation)
- `SIGMA_SYS_MUNMAP` - Unmap memory region

### Interrupt & Exception Handling
- `SIGMA_SYS_GET_TICKS` - Read kernel tick counter
- `SIGMA_SYS_SCHED_YIELD` - Yield CPU to next process

### Hardware Information
- `SIGMA_SYS_GET_CPU_INFO` - Get CPU vendor, model, features
- `SIGMA_SYS_GET_HARDWARE_INFO` - Get complete hardware specification

### Debug & Diagnostic
- `SIGMA_SYS_PRINTK` - Kernel debug output (for development)
- `SIGMA_SYS_GET_STATS` - Get kernel statistics

### Kernel Data Structures (for component use)
- `struct sigmaos_state` - Global kernel state
- `struct pcb` - Process control block
- `struct framebuffer` - Framebuffer information
- `struct hardware_info` - Hardware identification

## Test Structure (Arrange-Act-ASSERT)
All kernel tests follow the AAA (Arrange-Act-ASSERT) pattern:

1. **Arrange**: Set up test environment with specific kernel configuration
   - Initialize kernel subsystems
   - Set up memory regions
   - Configure interrupt handlers
   - Prepare test PCBs and process states

2. **Act**: Execute the code under test
   - Kernel syscall invocation
   - Interrupt handler registration/test
   - Memory allocation/deallocation
   - Scheduler testing
   - Paging table operations

3. **Assert**: Verify the expected kernel results
   - Return values match expected values
   - Memory consistency maintained
   - Interrupt state preserved/restored correctly
   - Process state transitions valid
   - No kernel panics or undefined behavior

### Example Test: Syscall Dispatch
```
Arrange: Initialize syscall dispatcher with valid function pointers
Act: Dispatch SIGMA_SYS_GETPID with no arguments
Assert: Returns current process ID in eax, carry flag clear
```

### Example Test: Memory Allocation
```
Arrange: Initialize kernel heap with available memory region
Act: Allocate memory block via kalloc(N) where N = 4 pages
Assert: Returns valid virtual address, heap pointer advances, subsequent alloc works
```

### Example Test: Interrupt Framework
```
Arrange: Register IRQ handler for timer interrupt
Act: Trigger timer interrupt via PIC
Assert: Handler executes, ticks increment, scheduler invoked
```

## Development
- Follow best practices with clear abstraction boundaries between kernel subsystems
- Kernel initialization is best-effort; gracefully degrades on unsupported configurations
- Comprehensive error types for all kernel failure scenarios (ENOMEM, EINVAL, EACCES, EPERM)
- Unit tests provided for kernel management and configuration via the test suite
- Integration tests for full SigmaOS system with actual kernel integration in CI
- Follow SigmaOS coding standards: single responsibility, defensive programming, fail-fast
- Use `#ifdef SIGMAOS_ARCH_X86_64` for architecture-specific code paths
- Modularize code so platform-specific portions are isolated (Linux-inspired conditional compilation)
- Document all kernel APIs with function signatures, return values, and error codes
- Maintain ABI stability for system call interface across kernel releases
- All kernel subsystems must pass `make hub-test` and `cargo test` before merging

## TODO
- Implement full GDT descriptor setup with proper segment limits and privileges
- Add complete IDT with all 256 exception and IRQ handlers (currently minimal)
- Implement proper PIT timer with divider programming and IRQ 0 handling
- Enable CPUID detection for CPU identification and feature detection
- Implement buddy allocator for kernel memory management (currently simple heap)
- Add multi-level page table initialization with proper page directory/level4 structures
- Implement ELF binary loading and execution support
- Add /dev interface for kernel services access
- Implement /proc and /sys filesystem interfaces for kernel information
- Add /kernel/debug interface for runtime kernel introspection
- Implement /kernel/log for kernel message ring buffer
- Add support for kernel module loading (inspired by Arch Linux modprobe)
- Implement /proc/stat, /proc/meminfo for system statistics
- Add ACPI table parsing and hardware configuration
- Implement proper power management interrupt (SMI, APIC timer)
- Add symmetric multiprocessing (SMP) support for multi-core systems
- Implement network stack foundation (packet scheduling, buffer management)
- Add filesystem VFS (Virtual File System) layer with ext2/ext3 support
- Implement kernel-side ELF interpreter for program execution
- Add /proc/interrupts for interrupt request statistics
- Implement /proc/mounts for mounted filesystem information
- Write comprehensive kernel documentation in docs/kernel/
- Add fuzz testing for system call interfaces
- Implement kernel self-tests that run at boot time
- Write kernel ABI stability guarantee document
- Add kernel crash dump / core dump support
- Implement /proc/kallsyms for kernel symbol table
- Add USB device kernel driver foundation
- Implement GPU/VGA initialization beyond text mode
- Write kernel licensing and compliance documentation