# Stands in for sshd's pam_motd, which the image drops, so both engines print the motd the same
# way: in interactive login shells only, never in commands, scp, sftp or Herdr's sessions.
case $- in
  *i*) cat /etc/motd ;;
esac
