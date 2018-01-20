package Local::PhiFD::TestNormalOps;

use strict;
use warnings;
use feature qw(say);
use Getopt::Long;
use List::MoreUtils qw(all);
use Scalar::Util qw(looks_like_number);
use IPC::Run qw(run timeout start harness);
use File::Spec;
use File::Path qw(make_path remove_tree);

use constant { TEST_NAME => 'test_normal_ops', };

sub new {
    my ( $class, %opts ) = @_;

    my $self = {};

    $self->{phifd} =
         $opts{phi_exec}
      || $ENV{PHIFD}
      || die 'Set PHIFD to the path to the FD binary.';

    $self->{test_runtime} = $opts{test_runtime} || 30;    # seconds
    die 'test_runtime must be a positive, integral number of seconds'
      unless $self->{test_runtime} > 0;

    $self->{ping_interval} = $opts{ping_interval} || 1;    # seconds
    die 'ping_interval must be a positive, integral number of seconds'
      unless $self->{ping_interval} > 0;

    $self->{log_root} = $opts{log_root} || 'logs/';
    $self->{test_name} = $opts{test_name};

    bless $self, $class;

    return $self;
}

do {

    my %opts;
    GetOptions( \%opts, 'ping_interval=i', 'test_runtime=i', 'phi_exec=s',
        'log_root_dir=s', )
      or die 'Invalid options found';

    __PACKAGE__->new( %opts, test_name => TEST_NAME )->main();

} unless (caller);

sub main {
    my ($self) = @_;

    local $SIG{__DIE__} = sub {
        my @args = @_;
        $self->cleanup;
        die @args;
    };

    $self->setup_logging();

    my @cmds = (
        [
            $self->{phifd}, '-t', $self->{ping_interval}, '-a',
            '127.0.0.1:12345'
        ],
        [
            $self->{phifd},    '-t', $self->{ping_interval}, '-a',
            '127.0.0.1:12346', '-i', '127.0.0.1:12345'
        ],
        [
            $self->{phifd},    '-t', $self->{ping_interval}, '-a',
            '127.0.0.1:12347', '-i', '127.0.0.1:12346'
        ],
    );

    my @harnesses;
    my @handles;
    for my $procnum ( 0 .. $#cmds ) {
        my $logfile = File::Spec->catfile( $self->test_log_dir,
            'proc_' . $procnum . '.log' );

        # XXX: do we need an :encoding(utf8) discipline here?
        open my $fh, '>', $logfile
          or die "Cannot open $logfile for writing: $!";

        my $harness = harness $cmds[$procnum], \undef, $fh, $fh, init => sub {
            $ENV{RUST_BACKTRACE} = 1;
        };

        push @handles,   $fh;
        push @harnesses, $harness;
    }

    my $start = time;
    while ( time - $start < $self->{test_runtime} ) {
        $_->pump_nb for @harnesses;
    }

    $self->cleanup();
}

sub cleanup {
    my ($self) = @_;
    $_->kill_kill for @{ $self->{harnesses} };
    close for @{ $self->{handles} };
}

sub setup_logging {
    my ($self) = @_;
    remove_tree( $self->test_log_dir );
    make_path( $self->test_log_dir );
}

sub test_log_dir {
    my ($self) = @_;
    return File::Spec->catfile( $self->{log_root}, $self->{test_name} );
}

1;
