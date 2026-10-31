use std::path::PathBuf;

use bande_volumes::mounted_volumes;

#[test]
fn removable_volumes_come_from_the_mount_table() {
    let mountinfo = "\
22 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw
98 22 8:1 / /run/media/ada/My\\040Stick rw,nosuid - vfat /dev/sda1 rw
99 22 8:2 / /run/media/bob/Other rw,nosuid - vfat /dev/sdb1 rw
100 22 8:3 / /media/ada/Camera rw,nosuid - exfat /dev/sdc1 rw
101 22 8:4 / /run/media/ada/deep/inside rw - ext4 /dev/sdd1 rw";
    let roots = vec![PathBuf::from("/run/media/ada"), PathBuf::from("/media/ada")];
    assert_eq!(
        mounted_volumes(mountinfo, &roots),
        vec![
            PathBuf::from("/media/ada/Camera"),
            PathBuf::from("/run/media/ada/My Stick"),
        ]
    );
}
