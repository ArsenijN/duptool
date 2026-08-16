#===============================================================================
#         Introducing the tester V1.0
#
# Q: What's that?
# A: It just tests that duptool make all work properly
#
# Q: How it works?
# A: Half of everything - automatically by this tester itself, other half - will require human to see the disk activity, or if I'll able to figure that out later how to do - will not require any human help
#===============================================================================


# Imports
from pathlib import Path
import os
import datetime

print(f"Welcome to the duptool tester V1!\n\nFirst, tester will check if duptool is there...")

#Starting with test that duptool exists
duptool_path = Path("./target/release/duptool.exe")
if duptool_path.is_file():
    duptool_md = os.path.getmtime(duptool_path)
    print(f"duptool is there! Modify date is {duptool_md} (from UNIX magic - {datetime.datetime.fromtimestamp(duptool_md)})")
else:
    print(f"It's not! Path {duptool_path} isn't valid or somehing else happened...")
    exit()

print(f"Since file is there, tester will run some tests...\n\n I'll ask the 2 drives to test some features that relate on how independant drives work. You'll need the Task Manager at the Disk Activity window to test what's going on. Also, I'll note that HDDs will be better since they're not that fast and you'll have enough time to see disk activity... I'll explain what you will need to do later\n\n")

drive1 = input("Please, provide drive 1 path (e.g. F:): ")
drive2 = input("Please, provide drive 2 path (e.g. G:): ")

print("Thanks! I'll check if those drives have ~1 GB of free space to test duptool...")

#TODO: drive free space check

print(f"Check is complete! Tester will create folder named \"duptool_tester_folder\" and then delete it after everything is done. Please be patient in some places...\n\n")

