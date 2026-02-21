# Monitoring CPU Wait Times for duptool

## Real-Time Monitoring Tools

### 1. `iotop` - Best for I/O Wait Analysis
Shows which processes are causing I/O wait:

```bash
# Install if needed:
sudo apt install iotop  # Debian/Ubuntu
sudo dnf install iotop  # Fedora

# Run with:
sudo iotop -o  # Only show processes doing I/O

# While running duptool, look for:
# - TID: Thread ID
# - DISK READ/WRITE: Actual I/O rates
# - IO%: Percentage of time waiting for I/O
# - COMMAND: Should show "duptool"
```

**Expected output:**
```
  TID  PRIO  USER     DISK READ  DISK WRITE  SWAPIN     IO>    COMMAND
 1234  be/4  user      50.00 M/s    0.00 B/s  0.00 %  5.32 %  duptool
 1235  be/4  user      45.00 M/s    0.00 B/s  0.00 %  4.87 %  duptool
```

### 2. `pidstat` - CPU Wait Per Thread
Shows detailed CPU stats per thread:

```bash
# Install if needed:
sudo apt install sysstat

# Monitor duptool while running:
pidstat -t -p $(pgrep duptool) 1

# Columns to watch:
# %usr: User CPU time (should be high)
# %system: System CPU time
# %wait: I/O wait time (should be low after fix)
# %CPU: Total CPU usage
```

**Good output (after fix):**
```
     TID    %usr %system  %wait    %CPU   CPU  Command
    1234   75.00    5.00   3.00   83.00     0  duptool
    1235   72.00    6.00   4.00   82.00     1  duptool
    1236   74.00    5.00   2.00   81.00     2  duptool
```

**Bad output (before fix):**
```
     TID    %usr %system  %wait    %CPU   CPU  Command
    1234   25.00    5.00  45.00   75.00     0  duptool
    1235   30.00    4.00  48.00   82.00     1  duptool
    1236   28.00    6.00  50.00   84.00     2  duptool
```

### 3. `perf` - Detailed Profiling
Most powerful tool for understanding what's blocking:

```bash
# Install:
sudo apt install linux-tools-common linux-tools-generic

# Profile duptool:
sudo perf record -g -p $(pgrep duptool)
# Let it run for 10-30 seconds, then Ctrl+C

# Analyze:
sudo perf report

# Look for:
# - High percentages in "mutex_lock" or "futex_wait" = lock contention
# - High percentages in "io_schedule" = actual I/O wait
# - High percentages in "md5_*" or file reading = good, threads working
```

### 4. `strace` - System Call Analysis
See what syscalls are blocking:

```bash
# Trace all threads:
strace -f -c -p $(pgrep duptool)
# Let it run, then Ctrl+C for summary

# Look for:
# - Many "futex" calls = lock contention
# - Many small "read" calls = buffer too small
# - Time spent in I/O syscalls
```

### 5. Simple `top` with Thread View
Quick built-in monitoring:

```bash
# Start top:
top

# Press 'H' to show threads
# Press 'P' to sort by CPU usage
# Find duptool threads

# Look at columns:
# - %CPU: Should be high (80-95%)
# - wa: Should be low (< 10%)
```

## Comparison Tests

### Before Fix (v0.1.9)
```bash
# Run duptool:
duptool -BDXE folder1 folder2 &

# In another terminal, immediately run:
pidstat -t -p $(pgrep duptool) 1 | tee before-fix.log

# Expected output in before-fix.log:
# Average %wait: 40-50%
# Average %usr: 25-35%
```

### After Fix (v0.1.10)
```bash
# Run duptool:
duptool -BDXE folder1 folder2 &

# Monitor:
pidstat -t -p $(pgrep duptool) 1 | tee after-fix.log

# Expected output in after-fix.log:
# Average %wait: 5-10%
# Average %usr: 70-85%
```

### Compare Results
```bash
# Average wait time before:
grep -v "^#" before-fix.log | awk '{sum+=$5; n++} END {print "Avg wait:", sum/n "%"}'

# Average wait time after:
grep -v "^#" after-fix.log | awk '{sum+=$5; n++} END {print "Avg wait:", sum/n "%"}'
```

## Quick Test Script

Save this as `monitor-duptool.sh`:

```bash
#!/bin/bash

echo "Starting duptool monitoring..."
echo "Make sure duptool is running!"
echo ""

# Find duptool PID
PID=$(pgrep duptool)

if [ -z "$PID" ]; then
    echo "ERROR: duptool is not running!"
    exit 1
fi

echo "Found duptool PID: $PID"
echo ""
echo "Monitoring for 30 seconds..."
echo "=========================="

# Monitor for 30 seconds
pidstat -t -p $PID 1 30 | tee duptool-stats.log

echo ""
echo "=========================="
echo "Analysis:"
echo ""

# Calculate averages
AVG_USR=$(grep -v "^#" duptool-stats.log | grep -v "Average" | awk '{sum+=$4; n++} END {if(n>0) print sum/n}')
AVG_WAIT=$(grep -v "^#" duptool-stats.log | grep -v "Average" | awk '{sum+=$6; n++} END {if(n>0) print sum/n}')
AVG_CPU=$(grep -v "^#" duptool-stats.log | grep -v "Average" | awk '{sum+=$8; n++} END {if(n>0) print sum/n}')

echo "Average User CPU:  ${AVG_USR}%"
echo "Average I/O Wait:  ${AVG_WAIT}%"
echo "Average Total CPU: ${AVG_CPU}%"
echo ""

if (( $(echo "$AVG_WAIT > 30" | bc -l) )); then
    echo "⚠️  HIGH I/O WAIT! (> 30%)"
    echo "   Possible causes:"
    echo "   - Lock contention (old code)"
    echo "   - Disk thrashing"
    echo "   - Slow USB drive"
elif (( $(echo "$AVG_WAIT > 15" | bc -l) )); then
    echo "⚠️  MODERATE I/O WAIT (15-30%)"
    echo "   Acceptable for HDD, but could be better"
elif (( $(echo "$AVG_WAIT < 15" | bc -l) )); then
    echo "✓ LOW I/O WAIT (< 15%)"
    echo "  Good performance!"
fi

echo ""
echo "Full log saved to: duptool-stats.log"
```

Make it executable and run:
```bash
chmod +x monitor-duptool.sh

# In terminal 1:
duptool -BDXE folder1 folder2

# In terminal 2:
./monitor-duptool.sh
```

## Understanding the Output

### Good Performance Indicators:
- ✓ I/O wait < 15%
- ✓ User CPU > 70%
- ✓ Few `futex` calls in strace
- ✓ Threads mostly in "R" (running) state

### Bad Performance Indicators:
- ❌ I/O wait > 30%
- ❌ User CPU < 40%
- ❌ Many `futex` calls in strace
- ❌ Threads in "D" (uninterruptible sleep) state

## Real-World Example

```bash
# Terminal 1: Run duptool
$ duptool -BDXE /media/usb/folder1 /mnt/ssd/folder2

# Terminal 2: Monitor
$ pidstat -t -p $(pgrep duptool) 1

# Output (GOOD - after fix):
Time        TID    %usr %system  %wait    %CPU   CPU  Command
15:23:45   1234   78.00    7.00   5.00   90.00     0  duptool
15:23:46   1234   82.00    6.00   4.00   92.00     0  duptool
15:23:47   1234   79.00    8.00   3.00   90.00     0  duptool
                   ^^^^           ^^^^
                   High!          Low!

# Output (BAD - before fix):
Time        TID    %usr %system  %wait    %CPU   CPU  Command
15:23:45   1234   35.00    5.00  48.00   88.00     0  duptool
15:23:46   1234   32.00    4.00  52.00   88.00     0  duptool
15:23:47   1234   38.00    6.00  45.00   89.00     0  duptool
                   ^^^^           ^^^^
                   Low!          High!
```

## Troubleshooting High Wait Times

If you still see high wait after the fix:

1. **Check if you rebuilt the binary:**
   ```bash
   strings target/release/duptool | grep "Local counters"
   # Should find the comment from new code
   ```

2. **Verify you're using the new binary:**
   ```bash
   ./target/release/duptool --version
   # Should show v0.1.10
   ```

3. **Check actual disk I/O:**
   ```bash
   sudo iotop -o
   # If DISK READ shows < 10 MB/s but wait is high → not disk I/O
   # If DISK READ shows > 50 MB/s → actual disk bottleneck
   ```

4. **Profile with perf:**
   ```bash
   sudo perf top -p $(pgrep duptool)
   # Look at top functions:
   # - High in "mutex_lock" → still lock contention
   # - High in "md5_compute" → good, actually working
   # - High in "io_schedule" → actual I/O wait
   ```
