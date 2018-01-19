use strict;
use warnings;
use feature qw(say);
use File::Slurp qw(read_file);
use Getopt::Long;

my %opts;
GetOptions(\%opts,
    'ping_interval=i',
    'tx_time=i',
) or die 'Invalid options received';

my @logfiles = @ARGV;

my $ping_interval = $opts{ping_interval} || 1; # seconds
my $tx_time = $opts{tx_time} // 0.001; # seconds

for my $who ( sort @logfiles ) {
    my @lines = read_file( $who );
    say $who;
    say('-' x 40);
    my $min = "inf";
    my $max = "-inf";
    my $sum = 0;
    my $sq_sum = 0;
    my $total = 0;
    for my $line ( @lines ) {
        if ( $line =~ /phi\((.*)\)=([^\s]+)\s*$/ ) {
            my ( $peer, $phi ) = ( $1, $2 );
            my ( $peer_ip_num, $peer_port ) = split /:/, $peer;
            my $ip = ip_number_to_ip_addr( $peer_ip_num );
            $min = $phi < $min ? $phi : $min;
            $max = $phi > $max ? $phi : $max;
            $sum += $phi;
            $sq_sum += $phi * $phi;
            ++$total;
        }
    }
    my $mean = $sum / $total;
    my $var = $sq_sum / $total - $mean * $mean;
    say "max: $max, min: $min, mean: $mean, var: $var";
}


sub ip_number_to_ip_addr {
    my $n = shift;
    return sprintf "%d.%d.%d.%d",
        (($n >> 24) & 0xff), (($n >> 16) & 0xff), (($n >> 8) & 0xff), ($n & 0xff);
}
